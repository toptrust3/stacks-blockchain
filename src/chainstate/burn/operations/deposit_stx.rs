use burnchains::{Burnchain, StacksHyperOp, StacksHyperOpType};
use chainstate::burn::db::sortdb::SortitionHandleTx;
use chainstate::burn::operations::leader_block_commit::RewardSetInfo;
use chainstate::burn::operations::DepositStxOp;
use chainstate::burn::operations::Error as op_error;
use clarity::types::chainstate::BurnchainHeaderHash;
use std::convert::TryFrom;

impl TryFrom<&StacksHyperOp> for DepositStxOp {
    type Error = op_error;

    fn try_from(value: &StacksHyperOp) -> Result<Self, Self::Error> {
        if let StacksHyperOpType::DepositStx {
            ref amount,
            ref sender,
        } = value.event
        {
            Ok(DepositStxOp {
                txid: value.txid.clone(),
                // use the StacksBlockId in the L1 event as the burnchain header hash
                burn_header_hash: BurnchainHeaderHash(value.in_block.0.clone()),
                amount: amount.clone(),
                sender: sender.clone(),
            })
        } else {
            Err(op_error::InvalidInput)
        }
    }
}

impl DepositStxOp {
    pub fn check(
        &self,
        _burnchain: &Burnchain,
        _tx: &mut SortitionHandleTx,
        _reward_set_info: Option<&RewardSetInfo>,
    ) -> Result<(), op_error> {
        // good to go!
        Ok(())
    }

    #[cfg(test)]
    pub fn set_burn_height(&mut self, _height: u64) {}
}