use crate::{
    fee::Fee,
    pool::Pool,
    token::{PairInfo, Token},
};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::Uint128;

#[cw_serde]
pub struct InstantiateMsg {
    pub router: String,
    pub pair: PairInfo,
    pub fee: Fee,
    pub execute: Option<ExecuteMsg>,
}

#[cw_serde]
pub enum ExecuteMsg {
    // Registers a new pool from a new chain to an already existing VLP
    RegisterPool {
        chain_id: String,
        factory: String,
        pair_info: PairInfo,
    },
    /*

    // Update the fee for the VLP
    UpdateFee {
        lp_fee: u64,
        treasury_fee: u64,
        staker_fee: u64,
    },
    */
}

#[cw_serde]
#[derive(QueryResponses)]

pub enum QueryMsg {
    // Query to simulate a swap for the asset
    #[returns(GetSwapResponse)]
    SimulateSwap { asset: Token, asset_amount: Uint128 },
    // Queries the total reserve of the pair in the VLP
    #[returns(GetLiquidityResponse)]
    Liquidity {},

    // Queries the fee of this specific pool
    #[returns(FeeResponse)]
    Fee {},

    // Queries the pool information for a chain id
    #[returns(PoolResponse)]
    Pool { chain_id: String },
    // Query to get all pools
    #[returns(AllPoolsResponse)]
    GetAllPools {},
}

// We define a custom struct for each query response
#[cw_serde]
pub struct GetSwapResponse {
    pub token_out: Uint128,
}

#[cw_serde]
pub struct GetLiquidityResponse {
    pub pair: PairInfo,
    pub token_1_reserve: Uint128,
    pub token_2_reserve: Uint128,
    pub total_lp_tokens: Uint128,
}

#[cw_serde]
pub struct PoolInfo {
    pub factory_address: String,
    pub chain: String,
    pub pool: Pool,
}
#[cw_serde]
pub struct FeeResponse {
    pub fee: Fee,
}

#[cw_serde]
pub struct PoolResponse {
    pub pool: Pool,
}

#[cw_serde]
pub struct AllPoolsResponse {
    pub pools: Vec<PoolInfo>,
}

#[cw_serde]
pub struct MigrateMsg {}
