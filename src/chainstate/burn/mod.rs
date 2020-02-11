/*
 copyright: (c) 2013-2018 by Blockstack PBC, a public benefit corporation.

 This file is part of Blockstack.

 Blockstack is free software. You may redistribute or modify
 it under the terms of the GNU General Public License as published by
 the Free Software Foundation, either version 3 of the License or
 (at your option) any later version.

 Blockstack is distributed in the hope that it will be useful,
 but WITHOUT ANY WARRANTY, including without the implied warranty of
 MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 GNU General Public License for more details.

 You should have received a copy of the GNU General Public License
 along with Blockstack. If not, see <http://www.gnu.org/licenses/>.
*/

/// This module contains the code for processing the burn chain state database

pub mod db;
pub mod distribution;
pub mod operations;
pub mod sortition;

pub const CONSENSUS_HASH_LIFETIME : u32 = 24;

use std::fmt;

use burnchains::Txid;
use burnchains::Address;
use burnchains::PublicKey;
use burnchains::BurnchainHeaderHash;

use util::vrf::VRFProof;
use util::hash::{Hash160, to_hex};

use sha2::Sha256;
use ripemd160::Ripemd160;

use rusqlite::Connection;
use rusqlite::Transaction;

use chainstate::burn::db::burndb::BurnDB;
use chainstate::burn::db::burndb::BurnDBTx;

use util::db::Error as db_error;

use core::SYSTEM_FORK_SET_VERSION;

use util::log;
use util::uint::Uint256;
use util::hash::Sha512Trunc256Sum;

use chainstate::stacks::index::TrieHash;

pub struct ConsensusHash(pub [u8; 20]);
impl_array_newtype!(ConsensusHash, u8, 20);
impl_array_hexstring_fmt!(ConsensusHash);
impl_byte_array_newtype!(ConsensusHash, u8, 20);
pub const CONSENSUS_HASH_ENCODED_SIZE : u32 = 20;

pub struct BlockHeaderHash(pub [u8; 32]);
impl_array_newtype!(BlockHeaderHash, u8, 32);
impl_array_hexstring_fmt!(BlockHeaderHash);
impl_byte_array_newtype!(BlockHeaderHash, u8, 32);
pub const BLOCK_HEADER_HASH_ENCODED_SIZE : usize = 32;

pub struct VRFSeed(pub [u8; 32]);
impl_array_newtype!(VRFSeed, u8, 32);
impl_array_hexstring_fmt!(VRFSeed);
impl_byte_array_newtype!(VRFSeed, u8, 32);
pub const VRF_SEED_ENCODED_SIZE : u32 = 32;

impl VRFSeed {
    /// First-ever VRF seed from the genesis block.  It's all 0's
    pub fn initial() -> VRFSeed {
        VRFSeed::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
    }

    pub fn from_proof(proof: &VRFProof) -> VRFSeed {
        let h = Sha512Trunc256Sum::from_data(&proof.to_bytes());
        VRFSeed(h.0)
    }

    pub fn is_from_proof(&self, proof: &VRFProof) -> bool {
        self.as_bytes().to_vec() == VRFSeed::from_proof(proof).as_bytes().to_vec()
    }
}

// operations hash -- the sha256 hash of a sequence of transaction IDs 
pub struct OpsHash(pub [u8; 32]);
impl_array_newtype!(OpsHash, u8, 32);
impl_array_hexstring_fmt!(OpsHash);
impl_byte_array_newtype!(OpsHash, u8, 32);

// rolling hash of PoW outputs to mix with the VRF seed on sortition 
pub struct SortitionHash(pub [u8; 32]);
impl_array_newtype!(SortitionHash, u8, 32);
impl_array_hexstring_fmt!(SortitionHash);
impl_byte_array_newtype!(SortitionHash, u8, 32);

#[derive(Debug, Clone, PartialEq)]
#[repr(u8)]
pub enum Opcodes {
    LeaderBlockCommit = '[' as u8,
    LeaderKeyRegister = '^' as u8,
    UserBurnSupport = '_' as u8
}

// a burnchain block snapshot
#[derive(Debug, Clone, PartialEq)]
pub struct BlockSnapshot {
    pub block_height: u64,
    pub burn_header_timestamp: u64,
    pub burn_header_hash: BurnchainHeaderHash,
    pub parent_burn_header_hash: BurnchainHeaderHash,
    pub consensus_hash: ConsensusHash,
    pub ops_hash: OpsHash,
    pub total_burn: u64,        // how many burn tokens have been destroyed since genesis
    pub sortition: bool,        // whether or not a sortition happened in this block (will be false if there were no burns)
    pub sortition_hash: SortitionHash,  // rolling hash of the burn chain's block headers -- this gets mixed with the sortition VRF seed
    pub winning_block_txid: Txid,       // txid of the leader block commit that won sortition.  Will all 0's if sortition is false. 
    pub winning_stacks_block_hash: BlockHeaderHash,     // hash of Stacks block that won sortition (will be all 0's if sortition is false)
    pub index_root: TrieHash,           // root hash of the index over the materialized view of all inserted data
    pub num_sortitions: u64,       // how many stacks blocks exist
}

impl BlockHeaderHash {
    pub fn to_hash160(&self) -> Hash160 {
        Hash160::from_sha256(&self.0)
    }

    pub fn from_serialized_header(buf: &[u8]) -> BlockHeaderHash {
        let h = Sha512Trunc256Sum::from_data(buf);
        let mut b = [0u8; 32];
        b.copy_from_slice(h.as_bytes());
        BlockHeaderHash(b)
    }
}

impl SortitionHash {
    /// Calculate a new sortition hash from the given burn header hash
    pub fn initial() -> SortitionHash {
        SortitionHash([0u8; 32])
    }

    /// Mix in a burn blockchain header to make a new sortition hash
    pub fn mix_burn_header(&self, burn_header_hash: &BurnchainHeaderHash) -> SortitionHash {
        use sha2::Digest;
        let mut sha2 = Sha256::new();
        sha2.input(self.as_bytes());
        sha2.input(burn_header_hash.as_bytes());
        let mut ret = [0u8; 32];
        ret.copy_from_slice(sha2.result().as_slice());
        SortitionHash(ret)
    }

    /// Mix in a new VRF seed to make a new sortition hash.
    pub fn mix_VRF_seed(&self, VRF_seed: &VRFSeed) -> SortitionHash {
        use sha2::Digest;
        let mut sha2 = Sha256::new();
        sha2.input(self.as_bytes());
        sha2.input(VRF_seed.as_bytes());
        let mut ret = [0u8; 32];
        ret.copy_from_slice(&sha2.result()[..]);
        SortitionHash(ret)
    }

    /// Convert a SortitionHash into a (little-endian) uint256
    pub fn to_uint256(&self) -> Uint256 {
        let mut tmp = [0u64; 4];
        for i in 0..4 {
            let b = (self.0[8*i] as u64) +
                    ((self.0[8*i + 1] as u64) << 8) +
                    ((self.0[8*i + 2] as u64) << 16) +
                    ((self.0[8*i + 3] as u64) << 24) +
                    ((self.0[8*i + 4] as u64) << 32) +
                    ((self.0[8*i + 5] as u64) << 40) +
                    ((self.0[8*i + 6] as u64) << 48) +
                    ((self.0[8*i + 7] as u64) << 56);

            tmp[i] = b;
        }
        Uint256(tmp)
    }
}

impl OpsHash {
    pub fn from_txids(txids: &Vec<Txid>) -> OpsHash {
        // NOTE: unlike stacks v1, we calculate the ops hash simply
        // from a hash-chain of txids.  There is no weird serialization
        // of operations, and we don't construct a merkle tree over
        // operations anymore (it's needlessly complex).
        use sha2::Digest;
        let mut hasher = Sha256::new();
        for txid in txids {
            hasher.input(txid.as_bytes());
        }
        let mut result_32 = [0u8; 32];
        result_32.copy_from_slice(hasher.result().as_slice());
        OpsHash(result_32)
    }
}

impl ConsensusHash {
    pub fn empty() -> ConsensusHash {
        ConsensusHash::from_hex("0000000000000000000000000000000000000000").unwrap()
    }

    /// Instantiate a consensus hash from this block's operations, the total burn so far
    /// for the resulting consensus hash, and the geometric series of previous consensus
    /// hashes.  Note that prev_consensus_hashes should be in order from most-recent to
    /// least-recent.
    pub fn from_ops(opshash: &OpsHash, total_burn: u64, prev_consensus_hashes: &Vec<ConsensusHash>) -> ConsensusHash {
        // NOTE: unlike stacks v1, we calculate the next consensus hash
        // simply as a hash-chain of the new ops hash, the sequence of 
        // previous consensus hashes, and the total burn that went into this
        // consensus hash.  We don't turn them into Merkle trees first.
        
        // encode the burn as a string, so it's unambiguous regardless of architecture endianness
        // (and it's not constrained by the word size)
        let burn_str = format!("{}", total_burn);
        assert!(burn_str.is_ascii());

        let result;
        {
            use sha2::Digest;
            let mut hasher = Sha256::new();

            // fork-set version... 
            hasher.input(SYSTEM_FORK_SET_VERSION);

            // ops hash...
            hasher.input(opshash.as_bytes());
            
            // total burn amount on this fork...
            hasher.input(burn_str.as_str().as_bytes());

            // previous consensus hashes...
            for ch in prev_consensus_hashes {
                hasher.input(ch.as_bytes());
            }

            result = hasher.result();
        }

        use ripemd160::Digest;
        let mut r160 = Ripemd160::new();
        r160.input(&result);
        
        let mut ch_bytes = [0u8; 20];
        ch_bytes.copy_from_slice(r160.result().as_slice());
        ConsensusHash(ch_bytes)
    }

    /// Get the previous consensus hashes that must be hashed to find
    /// the *next* consensus hash at a particular block.
    pub fn get_prev_consensus_hashes<'a>(tx: &mut BurnDBTx<'a>, block_height: u64, first_block_height: u64, tip_block_hash: &BurnchainHeaderHash) -> Result<Vec<ConsensusHash>, db_error> {
        let mut i = 0;
        let mut prev_chs = vec![];
        while i < 64 && block_height - (((1 as u64) << i) - 1) >= first_block_height {
            let prev_block : u64 = block_height - (((1 as u64) << i) - 1);
            let prev_ch = BurnDB::get_consensus_at(tx, prev_block, tip_block_hash)
                .expect(&format!("FATAL: failed to get consensus hash at {} in fork {}", prev_block, tip_block_hash));

            debug!("Consensus at {}: {}", prev_block, &prev_ch);
            prev_chs.push(prev_ch.clone());
            i += 1;

            if block_height < (((1 as u64) << i) - 1) {
                break;
            }
        }
        if i == 64 {
            // won't happen for a long, long time 
            panic!("FATAL ERROR: numeric overflow when calculating a consensus hash for {} from genesis block height {}", block_height, first_block_height);
        }

        Ok(prev_chs)
    }

    /// Make a new consensus hash, given the ops hash and parent block data
    pub fn from_parent_block_data<'a>(tx: &mut BurnDBTx<'a>, opshash: &OpsHash, parent_block_height: u64, first_block_height: u64, parent_block_hash: &BurnchainHeaderHash, total_burn: u64) -> Result<ConsensusHash, db_error> {
        let prev_consensus_hashes = ConsensusHash::get_prev_consensus_hashes(tx, parent_block_height, first_block_height, parent_block_hash)?;
        Ok(ConsensusHash::from_ops(opshash, total_burn, &prev_consensus_hashes))
    }

    /// raw consensus hash
    pub fn from_data(bytes: &[u8]) -> ConsensusHash {
        let result = {
            use sha2::Digest;
            let mut hasher = Sha256::new();
            hasher.input(bytes);
            hasher.result()
        };

        use ripemd160::Digest;
        let mut r160 = Ripemd160::new();
        r160.input(&result);
        
        let mut ch_bytes = [0u8; 20];
        ch_bytes.copy_from_slice(r160.result().as_slice());
        ConsensusHash(ch_bytes)
    }
}


#[cfg(test)]
mod tests {

    use super::*;

    use chainstate::burn::db::burndb::BurnDB;

    use burnchains::BurnchainHeaderHash;

    use burnchains::bitcoin::keys::BitcoinPublicKey;
    use burnchains::bitcoin::address::BitcoinAddress;

    use util::hash::{hex_bytes, Hash160};
    use util::log;
    use util::db::Error as db_error;

    use rusqlite::Connection;
    
    use util::get_epoch_time_secs;

    #[test]
    fn get_prev_consensus_hashes() {
        let first_burn_hash = BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap();
        let mut db = BurnDB::connect_memory(0, &first_burn_hash).unwrap();
        let mut burn_block_hashes = vec![];
        {
            let mut tx = db.tx_begin().unwrap();
            let mut prev_snapshot = BurnDB::get_first_block_snapshot(&tx).unwrap();
            burn_block_hashes.push(prev_snapshot.burn_header_hash.clone());
            for i in 1..256 {
                let snapshot_row = BlockSnapshot {
                    block_height: i,
                    burn_header_timestamp: get_epoch_time_secs(),
                    burn_header_hash: BurnchainHeaderHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,i as u8]).unwrap(),
                    parent_burn_header_hash: BurnchainHeaderHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,(if i == 0 { 0xff } else { i-1 }) as u8]).unwrap(),
                    consensus_hash: ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,i as u8]).unwrap(),
                    ops_hash: OpsHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,i as u8]).unwrap(),
                    total_burn: i,
                    sortition: true,
                    sortition_hash: SortitionHash::initial(),
                    winning_block_txid: Txid::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
                    winning_stacks_block_hash: BlockHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
                    index_root: TrieHash::from_empty_data(),     // will be overwritten
                    num_sortitions: i,
                };
                let next_index_root = BurnDB::append_chain_tip_snapshot(&mut tx, &prev_snapshot, &snapshot_row, &vec![], &vec![]).unwrap();
                burn_block_hashes.push(snapshot_row.burn_header_hash.clone());
                prev_snapshot = snapshot_row;
            }
            
            tx.commit().unwrap();
        }

        let mut tx = db.tx_begin().unwrap();
        
        let prev_chs_0 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 0, 0, &burn_block_hashes[0]).unwrap();
        assert_eq!(prev_chs_0, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]).unwrap()]);
        
        let prev_chs_1 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 1, 0, &burn_block_hashes[1]).unwrap();
        assert_eq!(prev_chs_1, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]).unwrap()]);
        
        let prev_chs_2 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 2, 0, &burn_block_hashes[2]).unwrap();
        assert_eq!(prev_chs_2, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]).unwrap()]);
        
        let prev_chs_3 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 3, 0, &burn_block_hashes[3]).unwrap();
        assert_eq!(prev_chs_3, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,3]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]).unwrap()]);
        
        let prev_chs_4 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 4, 0, &burn_block_hashes[4]).unwrap();
        assert_eq!(prev_chs_4, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,4]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,3]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]).unwrap()]);
        
        let prev_chs_5 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 5, 0, &burn_block_hashes[5]).unwrap();
        assert_eq!(prev_chs_5, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,5]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,4]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,2]).unwrap()]);
        
        let prev_chs_6 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 6, 0, &burn_block_hashes[6]).unwrap();
        assert_eq!(prev_chs_6, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,6]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,5]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,3]).unwrap()]);
        
        let prev_chs_7 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 7, 0, &burn_block_hashes[7]).unwrap();
        assert_eq!(prev_chs_7, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,7]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,6]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,4]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]).unwrap()]);
        
        let prev_chs_8 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 8, 0, &burn_block_hashes[8]).unwrap();
        assert_eq!(prev_chs_8, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,8]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,7]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,5]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]).unwrap()]);
        
        let prev_chs_62 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 62, 0, &burn_block_hashes[62]).unwrap();
        assert_eq!(prev_chs_62, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,62]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,61]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,59]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,55]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,47]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,31]).unwrap()]);

        let prev_chs_63 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 63, 0, &burn_block_hashes[63]).unwrap();
        assert_eq!(prev_chs_63, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,63]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,62]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,60]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,56]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,48]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,32]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]).unwrap()]);

        let prev_chs_64 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 64, 0, &burn_block_hashes[64]).unwrap();
        assert_eq!(prev_chs_64, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,64]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,63]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,61]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,57]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,49]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,33]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]).unwrap()]);

        let prev_chs_126 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 126, 0, &burn_block_hashes[126]).unwrap();
        assert_eq!(prev_chs_126, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,126]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,125]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,123]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,119]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,111]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,95]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,63]).unwrap()]);

        let prev_chs_127 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 127, 0, &burn_block_hashes[127]).unwrap();
        assert_eq!(prev_chs_127, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,127]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,126]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,124]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,120]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,112]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,96]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,64]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]).unwrap()]);

        let prev_chs_128 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 128, 0, &burn_block_hashes[128]).unwrap();
        assert_eq!(prev_chs_128, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,128]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,127]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,125]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,121]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,113]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,97]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,65]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]).unwrap()]);
        
        let prev_chs_254 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 254, 0, &burn_block_hashes[254]).unwrap();
        assert_eq!(prev_chs_254, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,254]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,253]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,251]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,247]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,239]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,223]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,191]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,127]).unwrap()]);

        let prev_chs_255 = ConsensusHash::get_prev_consensus_hashes(&mut tx, 255, 0, &burn_block_hashes[255]).unwrap();
        assert_eq!(prev_chs_255, vec![
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,255]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,254]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,252]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,248]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,240]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,224]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,192]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,128]).unwrap(),
            ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]).unwrap()]);
    }
}
