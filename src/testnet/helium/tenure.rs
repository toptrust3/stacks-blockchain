use super::{MemPool, MemPoolFS, Config};
use super::node::{SortitionedBlock, TESTNET_CHAIN_ID};

use std::thread;
use std::time;

use burnchains::{BurnchainHeaderHash, Txid};
use chainstate::stacks::db::{StacksChainState, StacksHeaderInfo, ClarityTx};
use chainstate::stacks::{StacksPrivateKey, StacksBlock, StacksWorkScore, StacksAddress, StacksTransactionSigner, StacksTransaction, TransactionVersion, StacksMicroblock, CoinbasePayload, StacksBlockBuilder, TransactionAnchorMode};
use chainstate::stacks::{MINER_BLOCK_BURN_HEADER_HASH, MINER_BLOCK_HEADER_HASH};
use chainstate::burn::{VRFSeed, BlockHeaderHash};
use util::vrf::{VRFProof};

pub struct TenureArtifacts {
    pub anchored_block: StacksBlock,
    pub microblocks: Vec<StacksMicroblock>,
    pub parent_block: SortitionedBlock,
    pub burn_fee: u64
}

pub struct Tenure {
    average_block_time: u64,
    block_builder: StacksBlockBuilder,
    coinbase_tx: StacksTransaction,
    config: Config,
    last_sortitioned_block: SortitionedBlock,
    pub mem_pool: MemPoolFS,
    parent_block: StacksHeaderInfo,
    started_at: std::time::Instant,
    pub vrf_seed: VRFSeed,
    burn_fee_cap: u64,
}

impl <'a> Tenure {

    pub fn new(parent_block: StacksHeaderInfo, 
               average_block_time: u64,
               coinbase_tx: StacksTransaction,
               config: Config,
               mem_pool: MemPoolFS,
               microblock_secret_key: StacksPrivateKey,  
               last_sortitioned_block: SortitionedBlock,
               vrf_proof: VRFProof,
               burn_fee_cap: u64) -> Tenure {

        let now = time::Instant::now();

        let ratio = StacksWorkScore {
            burn: last_sortitioned_block.total_burn,
            work: parent_block.anchored_header.total_work.work + 1,
        };

        let block_builder = match last_sortitioned_block.total_burn {
            0 => StacksBlockBuilder::first(1, &parent_block.burn_header_hash, parent_block.burn_header_timestamp, &vrf_proof, &microblock_secret_key),
            _ => StacksBlockBuilder::from_parent(1, &parent_block, &ratio, &vrf_proof, &microblock_secret_key)
        };

        Self {
            average_block_time,
            block_builder,
            coinbase_tx,
            config,
            last_sortitioned_block,
            mem_pool,
            parent_block,
            started_at: now,
            vrf_seed: VRFSeed::from_proof(&vrf_proof),
            burn_fee_cap,
        }
    }

    pub fn handle_txs(&mut self, clarity_tx: &mut ClarityTx<'a>, txs: Vec<StacksTransaction>) {
        for tx in txs {
            let res = self.block_builder.try_mine_tx(clarity_tx, &tx);
            match res {
                Err(e) => error!("Failed mining transaction - {}", e),
                Ok(_) => {},
            };
        }
    }

    pub fn run(&mut self) -> Option<TenureArtifacts> {

        let mut chain_state = StacksChainState::open(
            false, 
            TESTNET_CHAIN_ID, 
            &self.config.get_chainstate_path()).unwrap();

        let mut clarity_tx = self.block_builder.epoch_begin(&mut chain_state).unwrap();

        self.handle_txs(&mut clarity_tx, vec![self.coinbase_tx.clone()]);

        let txs = self.mem_pool.poll();
        self.handle_txs(&mut clarity_tx, txs);

        let anchored_block = self.block_builder.mine_anchored_block(&mut clarity_tx);

        clarity_tx.rollback_block();

        let artifact = TenureArtifacts {
            anchored_block,
            microblocks: vec![],
            parent_block: self.last_sortitioned_block.clone(),
            burn_fee: self.burn_fee_cap
        };
        Some(artifact)
    }
}
