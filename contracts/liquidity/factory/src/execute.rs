use cosmwasm_std::{
    ensure, to_json_binary, CosmosMsg, Decimal, DepsMut, Env, IbcMsg, IbcTimeout, MessageInfo,
    Response, SubMsg, Uint128, WasmMsg,
};
use cw20::Cw20ReceiveMsg;
use euclid::{
    chain::{CrossChainUser, CrossChainUserWithLimit},
    error::ContractError,
    events::{swap_event, tx_event},
    fee::{PartnerFee, MAX_PARTNER_FEE_BPS},
    liquidity::{AddLiquidityRequest, RemoveLiquidityRequest},
    pool::PoolCreateRequest,
    swap::{NextSwapPair, SwapRequest},
    timeout::get_timeout,
    token::{Pair, PairWithDenom, Token, TokenWithDenom},
};
use euclid_ibc::msg::{ChainIbcExecuteMsg, ChainIbcRemoveLiquidityExecuteMsg};

use crate::state::{
    HUB_CHANNEL, PAIR_TO_VLP, PENDING_ADD_LIQUIDITY, PENDING_POOL_REQUESTS,
    PENDING_REMOVE_LIQUIDITY, PENDING_SWAPS, STATE, TOKEN_TO_ESCROW,
};

pub fn execute_update_hub_channel(
    deps: DepsMut,
    info: MessageInfo,
    new_channel: String,
) -> Result<Response, ContractError> {
    let state = STATE.load(deps.storage)?;
    ensure!(info.sender == state.admin, ContractError::Unauthorized {});
    let old_channel = HUB_CHANNEL.may_load(deps.storage)?;
    HUB_CHANNEL.save(deps.storage, &new_channel)?;
    Ok(Response::new()
        .add_attribute("method", "execute_update_hub_channel")
        .add_attribute("new_channel", new_channel)
        .add_attribute("old_channel", old_channel.unwrap_or_default()))
}

// Function to send IBC request to Router in VLS to create a new pool
pub fn execute_request_pool_creation(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    pair: PairWithDenom,
    timeout: Option<u64>,
    tx_id: String,
) -> Result<Response, ContractError> {
    ensure!(
        !PENDING_POOL_REQUESTS.has(deps.storage, (info.sender.clone(), tx_id.clone())),
        ContractError::TxAlreadyExist {}
    );
    ensure!(
        !PAIR_TO_VLP.has(deps.storage, pair.get_pair()?.get_tupple()),
        ContractError::PoolAlreadyExists {}
    );

    let state = STATE.load(deps.storage)?;
    let sender = CrossChainUser {
        address: info.sender.to_string(),
        chain_uid: state.chain_uid,
    };

    let channel = HUB_CHANNEL.load(deps.storage)?;
    let timeout = get_timeout(timeout)?;

    let req = PoolCreateRequest {
        tx_id: tx_id.clone(),
        sender: info.sender.to_string(),
        pair_info: pair.clone(),
    };

    PENDING_POOL_REQUESTS.save(deps.storage, (info.sender.clone(), tx_id.clone()), &req)?;

    // Create IBC packet to send to Router
    let ibc_packet = IbcMsg::SendPacket {
        channel_id: channel.clone(),
        data: to_json_binary(&ChainIbcExecuteMsg::RequestPoolCreation {
            pair: pair.get_pair()?,
            sender,
            tx_id: tx_id.clone(),
        })?,
        timeout: IbcTimeout::with_timestamp(env.block.time.plus_seconds(timeout)),
    };

    Ok(Response::new()
        .add_event(tx_event(
            &tx_id,
            info.sender.as_str(),
            euclid::events::TxType::PoolCreation,
        ))
        .add_attribute("method", "request_pool_creation")
        .add_message(ibc_packet))
}

// Add liquidity to the pool
// TODO look into alternatives of using .branch(), maybe unifying the functions would help
pub fn add_liquidity_request(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    pair_info: PairWithDenom,
    token_1_liquidity: Uint128,
    token_2_liquidity: Uint128,
    slippage_tolerance: u64,
    timeout: Option<u64>,
    tx_id: String,
) -> Result<Response, ContractError> {
    // Check that slippage tolerance is between 1 and 100
    ensure!(
        (1..=100).contains(&slippage_tolerance),
        ContractError::InvalidSlippageTolerance {}
    );

    ensure!(
        !PENDING_ADD_LIQUIDITY.has(deps.storage, (info.sender.clone(), tx_id.clone())),
        ContractError::TxAlreadyExist {}
    );

    let state = STATE.load(deps.storage)?;
    let sender = CrossChainUser {
        address: info.sender.to_string(),
        chain_uid: state.chain_uid,
    };

    let pair = pair_info.get_pair()?;

    ensure!(
        PAIR_TO_VLP.has(deps.storage, pair.get_tupple()),
        ContractError::Generic {
            err: "Pool doesn't exist".to_string()
        }
    );

    let channel = HUB_CHANNEL.load(deps.storage)?;
    let timeout = get_timeout(timeout)?;

    // Check that the liquidity is greater than 0
    ensure!(
        !(token_1_liquidity.is_zero() || token_2_liquidity.is_zero()),
        ContractError::ZeroAssetAmount {}
    );

    // Do an early check for tokens escrow so that if it exists, it should allow the denom that we are sending
    let tokens = pair_info.get_vec_token_info();
    for token in tokens {
        let escrow_address = TOKEN_TO_ESCROW
            .load(deps.storage, token.token)
            .or(Err(ContractError::EscrowDoesNotExist {}))?;
        let token_allowed_query_msg = euclid::msgs::escrow::QueryMsg::TokenAllowed {
            denom: token.token_type,
        };
        let token_allowed: euclid::msgs::escrow::AllowedTokenResponse = deps
            .querier
            .query_wasm_smart(escrow_address.clone(), &token_allowed_query_msg)?;

        ensure!(
            token_allowed.allowed,
            ContractError::UnsupportedDenomination {}
        );
    }

    // Get the token 1 and token 2 from the pair info
    let token_1 = pair_info.token_1.clone();
    let token_2 = pair_info.token_2.clone();

    // Prepare msg vector
    let mut msgs: Vec<CosmosMsg> = Vec::new();

    // IF TOKEN IS A SMART CONTRACT IT REQUIRES APPROVAL FOR TRANSFER
    if token_1.token_type.is_smart() {
        let msg = token_1
            .token_type
            .create_transfer_msg(token_1_liquidity, env.contract.address.clone().to_string())?;
        msgs.push(msg);
    } else {
        // If funds empty return error
        ensure!(
            !info.funds.is_empty(),
            ContractError::InsufficientDeposit {}
        );

        // Check for funds sent with the message
        let amt = info
            .funds
            .iter()
            .find(|x| x.denom == token_1.token_type.get_denom())
            .ok_or(ContractError::Generic {
                err: "Denom not found".to_string(),
            })?;

        ensure!(
            amt.amount.ge(&token_1_liquidity),
            ContractError::InsufficientDeposit {}
        );
    }

    // Same for token 2
    if token_2.token_type.is_smart() {
        let msg = token_2
            .token_type
            .create_transfer_msg(token_2_liquidity, env.contract.address.clone().to_string())?;
        msgs.push(msg);
    } else {
        // If funds empty return error
        ensure!(
            !info.funds.is_empty(),
            ContractError::InsufficientDeposit {}
        );

        let amt = info
            .funds
            .iter()
            .find(|x| x.denom == token_2.token_type.get_denom())
            .ok_or(ContractError::Generic {
                err: "Denom not found".to_string(),
            })?;

        ensure!(
            amt.amount.ge(&token_2_liquidity),
            ContractError::InsufficientDeposit {}
        );
    }

    let timeout = IbcTimeout::with_timestamp(env.block.time.plus_seconds(timeout));

    let liquidity_tx_info = AddLiquidityRequest {
        sender: info.sender.to_string(),
        token_1_liquidity,
        token_2_liquidity,
        pair_info,
        tx_id: tx_id.clone(),
    };

    PENDING_ADD_LIQUIDITY.save(
        deps.storage,
        (info.sender.clone(), tx_id.clone()),
        &liquidity_tx_info,
    )?;

    // Create IBC packet to send to Router
    let ibc_packet = IbcMsg::SendPacket {
        channel_id: channel.clone(),
        data: to_json_binary(&ChainIbcExecuteMsg::AddLiquidity {
            sender,
            token_1_liquidity,
            token_2_liquidity,
            slippage_tolerance,
            pair,
            tx_id: tx_id.clone(),
        })?,
        timeout,
    };

    msgs.push(CosmosMsg::Ibc(ibc_packet));

    Ok(Response::new()
        .add_event(tx_event(
            &tx_id,
            info.sender.as_str(),
            euclid::events::TxType::AddLiquidity,
        ))
        .add_attribute("method", "add_liquidity_request")
        .add_messages(msgs))
}

// Add liquidity to the pool
// TODO look into alternatives of using .branch(), maybe unifying the functions would help
pub fn remove_liquidity_request(
    deps: DepsMut,
    info: MessageInfo,
    env: Env,
    pair: Pair,
    lp_allocation: Uint128,
    timeout: Option<u64>,
    mut cross_chain_addresses: Vec<CrossChainUserWithLimit>,
    tx_id: String,
) -> Result<Response, ContractError> {
    pair.validate()?;
    ensure!(
        !PENDING_REMOVE_LIQUIDITY.has(deps.storage, (info.sender.clone(), tx_id.clone())),
        ContractError::TxAlreadyExist {}
    );

    let state = STATE.load(deps.storage)?;
    let sender = CrossChainUser {
        address: info.sender.to_string(),
        chain_uid: state.chain_uid,
    };

    cross_chain_addresses.push(CrossChainUserWithLimit {
        user: sender.clone(),
        limit: None,
    });

    ensure!(
        PAIR_TO_VLP.has(deps.storage, pair.get_tupple()),
        ContractError::Generic {
            err: "Pool doesn't exist".to_string()
        }
    );

    let channel = HUB_CHANNEL.load(deps.storage)?;
    let timeout = get_timeout(timeout)?;

    // Check that the liquidity is greater than 0
    ensure!(!lp_allocation.is_zero(), ContractError::ZeroAssetAmount {});

    let timeout = IbcTimeout::with_timestamp(env.block.time.plus_seconds(timeout));

    let liquidity_tx_info = RemoveLiquidityRequest {
        sender: info.sender.to_string(),
        lp_allocation,
        pair: pair.clone(),
        tx_id: tx_id.clone(),
        cross_chain_addresses: cross_chain_addresses.clone(),
    };

    PENDING_REMOVE_LIQUIDITY.save(
        deps.storage,
        (info.sender.clone(), tx_id.clone()),
        &liquidity_tx_info,
    )?;

    // Create IBC packet to send to Router
    let ibc_packet = IbcMsg::SendPacket {
        channel_id: channel.clone(),
        data: to_json_binary(&ChainIbcExecuteMsg::RemoveLiquidity(
            ChainIbcRemoveLiquidityExecuteMsg {
                sender,
                lp_allocation,
                pair,
                cross_chain_addresses,
                tx_id: tx_id.clone(),
            },
        ))?,
        timeout,
    };

    let msg = CosmosMsg::Ibc(ibc_packet);

    Ok(Response::new()
        .add_event(tx_event(
            &tx_id,
            info.sender.as_str(),
            euclid::events::TxType::RemoveLiquidity,
        ))
        .add_attribute("method", "remove_liquidity_request")
        .add_message(msg))
}

// TODO make execute_swap an internal function OR merge execute_swap_request and execute_swap into one function

pub fn execute_swap_request(
    deps: &mut DepsMut,
    info: MessageInfo,
    env: Env,
    asset_in: TokenWithDenom,
    asset_out: Token,
    amount_in: Uint128,
    min_amount_out: Uint128,
    swaps: Vec<NextSwapPair>,
    timeout: Option<u64>,
    mut cross_chain_addresses: Vec<CrossChainUserWithLimit>,
    tx_id: String,
    partner_fee: Option<PartnerFee>,
) -> Result<Response, ContractError> {
    let state = STATE.load(deps.storage)?;
    let sender = CrossChainUser {
        address: info.sender.to_string(),
        chain_uid: state.chain_uid,
    };
    cross_chain_addresses.push(CrossChainUserWithLimit {
        user: sender.clone(),
        limit: None,
    });

    let partner_fee_bps = partner_fee
        .clone()
        .map(|fee| fee.partner_fee_bps)
        .unwrap_or(0);

    ensure!(
        partner_fee_bps <= MAX_PARTNER_FEE_BPS,
        ContractError::new("Invalid partner fee")
    );

    let partner_fee_amount = amount_in.checked_mul_ceil(Decimal::bps(partner_fee_bps))?;

    let amount_in = amount_in.checked_sub(partner_fee_amount)?;
    // Verify that the asset amount is greater than 0
    ensure!(!amount_in.is_zero(), ContractError::ZeroAssetAmount {});

    // Verify that the min amount out is greater than 0
    ensure!(!min_amount_out.is_zero(), ContractError::ZeroAssetAmount {});

    ensure!(
        !PENDING_SWAPS.has(deps.storage, (info.sender.clone(), tx_id.clone())),
        ContractError::TxAlreadyExist {}
    );

    let first_swap = swaps.first().ok_or(ContractError::Generic {
        err: "Empty Swap not allowed".to_string(),
    })?;

    ensure!(
        first_swap.token_in == asset_in.token,
        ContractError::new("Amount in doesn't match swap route")
    );

    let last_swap = swaps.last().ok_or(ContractError::Generic {
        err: "Empty Swap not allowed".to_string(),
    })?;

    ensure!(
        last_swap.token_out == asset_out,
        ContractError::new("Amount out doesn't match swap route")
    );

    let channel = HUB_CHANNEL.load(deps.storage)?;
    let timeout = get_timeout(timeout)?;
    let timeout = IbcTimeout::with_timestamp(env.block.time.plus_seconds(timeout));

    // Verify that this asset is allowed
    let escrow = TOKEN_TO_ESCROW.load(deps.storage, asset_in.token.clone())?;

    let token_allowed: euclid::msgs::escrow::AllowedTokenResponse = deps.querier.query_wasm_smart(
        escrow,
        &euclid::msgs::escrow::QueryMsg::TokenAllowed {
            denom: asset_in.token_type.clone(),
        },
    )?;

    ensure!(
        token_allowed.allowed,
        ContractError::UnsupportedDenomination {}
    );

    // Verify if the token is native
    if asset_in.token_type.is_native() {
        // Get the denom of native token
        let denom = asset_in.token_type.get_denom();

        // Verify thatthe amount of funds passed is greater than the asset amount
        if info
            .funds
            .iter()
            .find(|x| x.denom == denom)
            .ok_or(ContractError::Generic {
                err: "Denom not found".to_string(),
            })?
            .amount
            < amount_in
        {
            return Err(ContractError::Generic {
                err: "Funds attached are less than funds needed".to_string(),
            });
        }
    } else {
        // Verify that the contract address is the same as the asset contract address
        ensure!(
            info.sender == asset_in.token_type.get_denom(),
            ContractError::Unauthorized {}
        );
    }
    let swap_info = SwapRequest {
        sender: info.sender.to_string(),
        asset_in: asset_in.clone(),
        asset_out: asset_out.clone(),
        amount_in,
        min_amount_out,
        swaps: swaps.clone(),
        timeout: timeout.clone(),
        tx_id: tx_id.clone(),
        cross_chain_addresses: cross_chain_addresses.clone(),
        partner_fee_amount,
        partner_fee_recipient: partner_fee
            .map(|partner_fee| deps.api.addr_validate(&partner_fee.recipient))
            .transpose()?,
    };
    PENDING_SWAPS.save(
        deps.storage,
        (info.sender.clone(), tx_id.clone()),
        &swap_info,
    )?;

    // Create IBC packet to send to Router
    let ibc_packet = IbcMsg::SendPacket {
        channel_id: channel.clone(),
        data: to_json_binary(&ChainIbcExecuteMsg::Swap(
            euclid_ibc::msg::ChainIbcSwapExecuteMsg {
                sender,
                asset_in: asset_in.token,
                amount_in,
                asset_out,
                min_amount_out,
                swaps,
                tx_id: tx_id.clone(),
                cross_chain_addresses,
            },
        ))?,
        timeout,
    };

    let msg = CosmosMsg::Ibc(ibc_packet);

    Ok(Response::new()
        .add_event(tx_event(
            &tx_id,
            info.sender.as_str(),
            euclid::events::TxType::Swap,
        ))
        .add_event(swap_event(&tx_id, &swap_info))
        .add_attribute("method", "execute_request_swap")
        .add_message(msg))
}

/// Receives a message of type [`Cw20ReceiveMsg`] and processes it depending on the received template.
///
/// * **cw20_msg** is the CW20 message that has to be processed.
pub fn receive_cw20(
    _deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    _cw20_msg: Cw20ReceiveMsg,
) -> Result<Response, ContractError> {
    // match from_json(&cw20_msg.msg)? {
    //     // Allow to swap using a CW20 hook message
    //     Cw20HookMsg::Swap {
    //         asset,
    //         min_amount_out,
    //         timeout,
    //     } => {
    //         let contract_adr = info.sender.clone();

    //         // ensure that contract address is same as asset being swapped
    //         ensure!(
    //             contract_adr == asset.get_contract_address(),
    //             ContractError::AssetDoesNotExist {}
    //         );
    //         // Add sender as the option

    //         // ensure that the contract address is the same as the asset contract address
    //         execute_swap_request(
    //             &mut deps,
    //             info,
    //             env,
    //             asset,
    //             cw20_msg.amount,
    //             min_amount_out,
    //             Some(cw20_msg.sender),
    //             timeout,
    //         )
    //     }
    //     Cw20HookMsg::Deposit {} => {}
    // }
    Err(ContractError::NotImplemented {})
}

// New factory functions //
pub fn execute_request_register_denom(
    deps: DepsMut,
    token: TokenWithDenom,
) -> Result<Response, ContractError> {
    let escrow_address = TOKEN_TO_ESCROW.load(deps.storage, token.token.clone());
    ensure!(escrow_address.is_ok(), ContractError::EscrowDoesNotExist {});

    let escrow_address = escrow_address?;

    let msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: escrow_address.into_string(),
        msg: to_json_binary(&euclid::msgs::escrow::ExecuteMsg::AddAllowedDenom {
            denom: token.token_type.clone(),
        })?,
        funds: vec![],
    });
    Ok(Response::new()
        .add_submessage(SubMsg::new(msg))
        .add_attribute("method", "request_add_allowed_denom")
        .add_attribute("token", token.token.to_string())
        .add_attribute("denom", token.token_type.get_key()))
}

pub fn execute_request_deregister_denom(
    deps: DepsMut,
    token: TokenWithDenom,
) -> Result<Response, ContractError> {
    let escrow_address = TOKEN_TO_ESCROW.load(deps.storage, token.token.clone());
    ensure!(escrow_address.is_ok(), ContractError::EscrowDoesNotExist {});
    let escrow_address = escrow_address?;

    let msg = CosmosMsg::Wasm(WasmMsg::Execute {
        contract_addr: escrow_address.into_string(),
        msg: to_json_binary(&euclid::msgs::escrow::ExecuteMsg::DisallowDenom {
            denom: token.token_type.clone(),
        })?,
        funds: vec![],
    });
    Ok(Response::new()
        .add_submessage(SubMsg::new(msg))
        .add_attribute("method", "request_disallow_denom")
        .add_attribute("token", token.token.to_string())
        .add_attribute("denom", token.token_type.get_key()))
}
