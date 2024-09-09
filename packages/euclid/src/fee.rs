use std::collections::HashMap;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Deps, Uint128};

use crate::chain::CrossChainUser;

// Set maximum fee as 10%
pub const MAX_FEE_BPS: u64 = 1000;
// Fee Config for a VLP contract
#[cw_serde]
pub struct Fee {
    // Fee for lp providers
    pub lp_fee_bps: u64,
    // Fee for euclid treasury, distributed among stakers and other euclid related rewards
    pub euclid_fee_bps: u64,
    // Recipient for the fee
    pub recipient: CrossChainUser,
}

#[cw_serde]
pub struct TotalFees {
    // Fee for lp providers
    pub lp_fees: DenomFees,
    // Fee for euclid treasury, distributed among stakers and other euclid related rewards
    pub euclid_fees: DenomFees,
}

#[cw_serde]
pub struct DenomFees {
    // A map to store the total fees per denomination
    pub totals: HashMap<String, Uint128>,
}

impl DenomFees {
    // Add or update the total for a given denomination
    pub fn add_fee(&mut self, token: String, amount: Uint128) {
        self.totals
            .entry(token)
            .and_modify(|total| *total += amount)
            .or_insert(amount);
    }
    // Get the total for a given denomination
    pub fn get_fee(&self, token: &str) -> Uint128 {
        self.totals.get(token).cloned().unwrap_or_default()
    }

    pub fn get_key(&self, deps: &Deps, denom: &str) -> String {
        let denom_type = deps.api.addr_validate(denom);
        match denom_type {
            // If it's a valid address, it's a smart contract
            Ok(_) => format!("smart{}", denom),
            // If it's not a valid address, it's a native token
            Err(_) => format!("native{}", denom),
        }
    }
}
// Set maximum fee as 0.3%
pub const MAX_PARTNER_FEE_BPS: u64 = 30;

// Fee Config for a VLP contract
#[cw_serde]
pub struct PartnerFee {
    // The percentage of the fee for platform - 0 to 1
    pub partner_fee_bps: u64,
    pub recipient: String,
}
