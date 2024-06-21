use cosmwasm_schema::cw_serde;
use cosmwasm_std::{IbcTimeout, Uint128};

use crate::{
    error::ContractError,
    token::{Token, TokenInfo},
};

// Struct that stores a certain swap info
#[cw_serde]
pub struct SwapInfo {
    // The asset being swapped
    pub asset_in: TokenInfo,
    // The asset being received
    pub asset_out: TokenInfo,
    // The amount of asset being swapped
    pub amount_in: Uint128,
    // The min amount of asset being received
    pub min_amount_out: Uint128,
    pub swaps: Vec<NextSwap>,
    // The timeout specified for the swap
    pub timeout: IbcTimeout,
    // The Swap Main Identifier
    pub swap_id: String,
}

#[cw_serde]
pub struct NextSwap {
    pub vlp_address: String,
}
impl Default for NextSwap {
    fn default() -> Self {
        NextSwap {
            vlp_address: Default::default(), // Initialize each field with its default value
                                             // Initialize other fields similarly
        }
    }
}

#[cw_serde]
pub struct SwapResponse {
    pub asset_in: Token,
    pub asset_out: Token,
    pub amount_in: Uint128,
    pub amount_out: Uint128,
    // Add Swap Unique Identifier
    pub swap_id: String,
}

#[cw_serde]
pub struct SwapExtractedId {
    pub sender: String,
    pub index: u128,
}

// Function to extract sender from swap_id
pub fn parse_swap_id(id: &str) -> Result<SwapExtractedId, ContractError> {
    let parsed: Vec<&str> = id.split('-').collect();
    Ok(SwapExtractedId {
        sender: parsed[0].to_string(),
        index: parsed[1].parse()?,
    })
}

#[cfg(test)]
mod tests {

    use super::*;
    // Name isn't being printed, but is useful as a title for each test case
    #[allow(dead_code)]
    struct TestGetSwapExtractedId {
        name: &'static str,
        id: &'static str,
        expected_error: Option<ContractError>,
        expected_result: Option<SwapExtractedId>,
    }

    #[test]
    fn test_parse_swap_id() {
        let test_cases = vec![
            TestGetSwapExtractedId {
                name: "ID with sender and count",
                id: "eucl-10",
                expected_error: None,
                expected_result: Some(SwapExtractedId {
                    sender: "eucl".to_string(),
                    index: 10_u128,
                }),
            },
            // Not having a sender does not error
            TestGetSwapExtractedId {
                name: "ID with empty sender",
                id: "-10",
                expected_error: None,
                expected_result: Some(SwapExtractedId {
                    sender: "".to_string(),
                    index: 10_u128,
                }),
            },
            // Not having a count results in an error
            TestGetSwapExtractedId {
                name: "ID with empty count",
                id: "eucl-",
                expected_error: Some(ContractError::InvalidChainId {}),
                expected_result: Some(SwapExtractedId {
                    sender: "".to_string(),
                    index: 10_u128,
                }),
            },
        ];

        for test in test_cases {
            let res = parse_swap_id(test.id);

            if let Some(_err) = test.expected_error {
                assert!(res.is_err());
                continue;
            } else {
                assert_eq!(res.unwrap(), test.expected_result.unwrap())
            }
        }
    }
}
