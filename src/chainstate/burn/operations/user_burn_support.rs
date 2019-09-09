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
use std::marker::PhantomData;

use chainstate::burn::operations::Error as op_error;
use chainstate::burn::ConsensusHash;
use chainstate::burn::Opcodes;
use chainstate::burn::BlockHeaderHash;
use chainstate::burn::db::burndb::BurnDB;

use chainstate::burn::operations::{
    LeaderBlockCommitOp,
    LeaderKeyRegisterOp,
    UserBurnSupportOp,
    BlockstackOperation,
    BlockstackOperationType,
    parse_u16_from_be
};

use burnchains::BurnchainBlockHeader;
use burnchains::BurnchainTransaction;
use burnchains::Txid;
use burnchains::Address;
use burnchains::PublicKey;
use burnchains::BurnchainHeaderHash;
use burnchains::Burnchain;

use util::hash::Hash160;
use util::vrf::{VRF, VRFPublicKey};
use util::log;

use util::db::DBConn;
use util::db::DBTx;

// return type for parse_data (below)
struct ParsedData {
    pub consensus_hash: ConsensusHash,
    pub public_key: VRFPublicKey,
    pub key_block_backptr: u16,
    pub key_vtxindex: u16,
    pub block_header_hash_160: Hash160,
    pub memo: Vec<u8>
}

impl UserBurnSupportOp {
    #[cfg(test)]
    pub fn new(public_key: &VRFPublicKey, key_block_height: u16, key_vtxindex: u16, block_hash: &BlockHeaderHash, burn_fee: u64) -> UserBurnSupportOp {
        UserBurnSupportOp {
            public_key: public_key.clone(),
            block_header_hash_160: Hash160::from_sha256(block_hash.as_bytes()),
            memo: vec![],
            burn_fee: burn_fee,
            key_vtxindex: key_vtxindex,

            // partially filled in
            key_block_backptr: key_block_height,
            
            // to be filled in 
            consensus_hash: ConsensusHash([0u8; 20]),
            txid: Txid([0u8; 32]),
            vtxindex: 0,
            block_height: 0,
            burn_header_hash: BurnchainHeaderHash([0u8; 32]),
            fork_segment_id: 0,
        }
    }

    #[cfg(test)]
    pub fn set_mined_at(&mut self, burnchain: &Burnchain, consensus_hash: &ConsensusHash, block_header: &BurnchainBlockHeader) -> () {
        if self.consensus_hash != ConsensusHash([0u8; 20]) {
            self.consensus_hash = consensus_hash.clone();
        }

        if self.txid != Txid([0u8; 32]) {
            self.txid = Txid::from_test_data(block_header.block_height, self.vtxindex, &block_header.block_hash);
        }
        
        if self.burn_header_hash != BurnchainHeaderHash([0u8; 32]) {
            self.burn_header_hash = block_header.block_hash.clone();
        }

        self.key_block_backptr = (block_header.block_height - (self.key_block_backptr as u64)) as u16;
        self.block_height = block_header.block_height;
        self.fork_segment_id = block_header.fork_segment_id;
    }

    fn parse_data(data: &Vec<u8>) -> Option<ParsedData> {
        /*
            Wire format:

            0      2  3              23                       55                 75       77        79    80
            |------|--|---------------|-----------------------|------------------|--------|---------|-----|
             magic  op consensus hash    proving public key       block hash 160   key blk  key      memo
                                                                                   backptr  vtxindex

            
             Note that `data` is missing the first 3 bytes -- the magic and op have been stripped
        */
        // memo can be empty, and magic + op are omitted 
        if data.len() < 77 {
            warn!("USER_BURN_SUPPORT payload is malformed ({} bytes)", data.len());
            return None;
        }

        let consensus_hash = ConsensusHash::from_vec(&data[0..20].to_vec()).expect("FATAL: invalid data slice for consensus hash");
        let pubkey = match VRFPublicKey::from_bytes(&data[20..52].to_vec()) {
            Some(pubk) => {
                pubk
            },
            None => {
                warn!("Invalid VRF public key");
                return None;
            }
        };

        let block_header_hash_160 = Hash160::from_vec(&data[52..72].to_vec()).expect("FATAL: invalid data slice for block hash160");
        let key_block_backptr = parse_u16_from_be(&data[72..74]).unwrap();
        let key_vtxindex = parse_u16_from_be(&data[74..76]).unwrap();

        let memo = data[76..].to_vec();

        Some(ParsedData {
            consensus_hash,
            public_key: pubkey,
            block_header_hash_160,
            key_block_backptr,
            key_vtxindex,
            memo
        })
    }

    fn parse_from_tx(block_height: u64, fork_segment_id: u64, block_hash: &BurnchainHeaderHash, tx: &BurnchainTransaction) -> Result<UserBurnSupportOp, op_error> {
        // can't be too careful...
        let inputs = tx.get_signers();
        let outputs = tx.get_recipients();

        if inputs.len() == 0 || outputs.len() == 0 {
            test_debug!("Invalid tx: inputs: {}, outputs: {}", inputs.len(), outputs.len());
            return Err(op_error::InvalidInput);
        }

        if tx.opcode() != Opcodes::UserBurnSupport as u8 {
            test_debug!("Invalid tx: invalid opcode {}", tx.opcode());
            return Err(op_error::InvalidInput);
        }

        // outputs[0] should be the burn output
        if !outputs[0].address.is_burn() {
            // wrong burn output
            test_debug!("Invalid tx: burn output missing (got {:?})", outputs[0]);
            return Err(op_error::ParseError);
        }

        let burn_fee = outputs[0].amount;

        let data = match UserBurnSupportOp::parse_data(&tx.data()) {
            None => {
                test_debug!("Invalid tx data");
                return Err(op_error::ParseError);
            },
            Some(d) => d
        };

        // basic sanity checks
        if data.key_block_backptr == 0 {
            warn!("Invalid tx: key block back-pointer must be positive");
            return Err(op_error::ParseError);
        }

        if data.key_block_backptr as u64 >= block_height {
            warn!("Invalid tx: key block back-pointer {} exceeds block height {}", data.key_block_backptr, block_height);
            return Err(op_error::ParseError);
        }

        Ok(UserBurnSupportOp {
            consensus_hash: data.consensus_hash,
            public_key: data.public_key,
            block_header_hash_160: data.block_header_hash_160,
            key_block_backptr: data.key_block_backptr,
            key_vtxindex: data.key_vtxindex,
            memo: data.memo,
            burn_fee: burn_fee,

            txid: tx.txid(),
            vtxindex: tx.vtxindex(),
            block_height: block_height,
            burn_header_hash: block_hash.clone(),
            fork_segment_id: fork_segment_id
        })
    }
}

impl BlockstackOperation for UserBurnSupportOp {
    fn from_tx(block_header: &BurnchainBlockHeader, tx: &BurnchainTransaction) -> Result<UserBurnSupportOp, op_error> {
        UserBurnSupportOp::parse_from_tx(block_header.block_height, block_header.fork_segment_id, &block_header.block_hash, tx)
    }

    fn check<'a>(&self, burnchain: &Burnchain, block_header: &BurnchainBlockHeader, tx: &mut DBTx<'a>) -> Result<(), op_error> {
        // this will be the chain tip we're building on
        let chain_tip = BurnDB::get_block_snapshot(tx, &block_header.parent_block_hash)
            .expect("FATAL: failed to query parent block snapshot")
            .expect("FATAL: no parent snapshot in the DB");

        let leader_key_block_height = self.block_height - (self.key_block_backptr as u64);

        /////////////////////////////////////////////////////////////////
        // Consensus hash must be recent and valid
        /////////////////////////////////////////////////////////////////

        let consensus_hash_recent = BurnDB::is_fresh_consensus_hash(tx, chain_tip.block_height, burnchain.consensus_hash_lifetime.into(), &self.consensus_hash, chain_tip.fork_segment_id)
            .expect("Sqlite failure while verifying that a consensus hash is fresh");

        if !consensus_hash_recent {
            warn!("Invalid user burn: invalid consensus hash {}", &self.consensus_hash.to_hex());
            return Err(op_error::UserBurnSupportBadConsensusHash);
        }

        /////////////////////////////////////////////////////////////////////////////////////
        // There must exist a previously-accepted LeaderKeyRegisterOp that matches this 
        // user support burn's VRF public key.
        /////////////////////////////////////////////////////////////////////////////////////
        if self.key_block_backptr == 0 {
            warn!("Invalid tx: key block back-pointer must be positive");
            return Err(op_error::ParseError);
        }

        if self.key_block_backptr as u64 >= self.block_height {
            warn!("Invalid tx: key block back-pointer {} exceeds block height {}", self.key_block_backptr, self.block_height);
            return Err(op_error::ParseError);
        }

        let register_key_opt = BurnDB::get_leader_key_at(tx, leader_key_block_height, self.key_vtxindex.into(), chain_tip.fork_segment_id)
            .expect("Sqlite failure while fetching a leader record by VRF key");

        if register_key_opt.is_none() {
            warn!("Invalid user burn: no such leader VRF key {}", &self.public_key.to_hex());
            return Err(op_error::UserBurnSupportNoLeaderKey);
        }
        
        /////////////////////////////////////////////////////////////////////////////////////
        // The block hash can't be checked here -- the corresponding LeaderBlockCommitOp may
        // not have been checked yet, so we don't know yet if it exists.  The sortition
        // algorithm will carry out this check, and only consider user burns if they match
        // a block commit and the commit's corresponding leader key.
        /////////////////////////////////////////////////////////////////////////////////////

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burnchains::bitcoin::blocks::BitcoinBlockParser;
    use burnchains::bitcoin::BitcoinNetworkType;
    use burnchains::Txid;
    use burnchains::BLOCKSTACK_MAGIC_MAINNET;
    use burnchains::BurnchainBlockHeader;

    use burnchains::bitcoin::keys::BitcoinPublicKey;
    use burnchains::bitcoin::address::BitcoinAddress;

    use chainstate::burn::operations::{
        LeaderBlockCommitOp,
        LeaderKeyRegisterOp,
        UserBurnSupportOp,
        BlockstackOperation,
        BlockstackOperationType
    };

    use chainstate::burn::{ConsensusHash, OpsHash, SortitionHash, BlockSnapshot};
    
    use chainstate::stacks::StacksAddress;
    
    use deps::bitcoin::network::serialize::deserialize;
    use deps::bitcoin::blockdata::transaction::Transaction;

    use util::hash::{hex_bytes, Hash160};
    use util::log;

    struct OpFixture {
        txstr: String,
        result: Option<UserBurnSupportOp>
    }

    struct CheckFixture {
        op: UserBurnSupportOp,
        res: Result<(), op_error>
    }

    fn make_tx(hex_str: &str) -> Result<Transaction, &'static str> {
        let tx_bin = hex_bytes(hex_str)
            .map_err(|_e| "failed to decode hex string")?;
        let tx = deserialize(&tx_bin.to_vec())
            .map_err(|_e| "failed to deserialize")?;
        Ok(tx)
    }

    #[test]
    fn test_parse() {
        let vtxindex = 1;
        let block_height = 694;
        let burn_header_hash = BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap();

        let tx_fixtures: Vec<OpFixture> = vec![
            OpFixture {
                txstr: "01000000011111111111111111111111111111111111111111111111111111111111111111000000006a47304402204c51707ac34b6dcbfc518ba40c5fc4ef737bf69cc21a9f8a8e6f621f511f78e002200caca0f102d5df509c045c4fe229d957aa7ef833dc8103dc2fe4db15a22bab9e012102d8015134d9db8178ac93acbc43170a2f20febba5087a5b0437058765ad5133d000000000030000000000000000536a4c5069645f2222222222222222222222222222222222222222a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a3333333333333333333333333333333333333333010203040539300000000000001976a914000000000000000000000000000000000000000088aca05b0000000000001976a9140be3e286a15ea85882761618e366586b5574100d88ac00000000".to_string(),
                result: Some(UserBurnSupportOp {
                    consensus_hash: ConsensusHash::from_bytes(&hex_bytes("2222222222222222222222222222222222222222").unwrap()).unwrap(),
                    public_key: VRFPublicKey::from_bytes(&hex_bytes("a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a").unwrap()).unwrap(),
                    block_header_hash_160: Hash160::from_bytes(&hex_bytes("3333333333333333333333333333333333333333").unwrap()).unwrap(),
                    key_block_backptr: 513,
                    key_vtxindex: 1027,
                    memo: vec![0x05],
                    burn_fee: 12345,

                    txid: Txid::from_bytes_be(&hex_bytes("1d5cbdd276495b07f0e0bf0181fa57c175b217bc35531b078d62fc20986c716c").unwrap()).unwrap(),
                    vtxindex: vtxindex,
                    block_height: block_height,
                    burn_header_hash: burn_header_hash,

                    fork_segment_id: 0
                })
            },
            OpFixture {
                // invalid -- no burn output
                txstr: "01000000011111111111111111111111111111111111111111111111111111111111111111000000006a473044022073490a3958b9e6128d3b7a4a8c77203c56862b2da382e96551f7efae7029b0e1022046672d1e61bdfd3dca9cc199bffd0bfb9323e432f8431bb6749da3c5bd06e9ca012102d8015134d9db8178ac93acbc43170a2f20febba5087a5b0437058765ad5133d000000000020000000000000000536a4c5069645f2222222222222222222222222222222222222222a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a33333333333333333333333333333333333333330102030405a05b0000000000001976a9140be3e286a15ea85882761618e366586b5574100d88ac00000000".to_string(),
                result: None,
            },
            OpFixture {
                // invalid -- bad public key
                txstr: "01000000011111111111111111111111111111111111111111111111111111111111111111000000006a47304402202bf944fa4d1dbbdd4f53e915c85f07c8a5afbf917f7cc9169e9c7d3bbadff05a022064b33a1020dd9cdd0ac6de213ee1bd8f364c9c876e716ad289f324c2a4bbe48a012102d8015134d9db8178ac93acbc43170a2f20febba5087a5b0437058765ad5133d000000000030000000000000000536a4c5069645f2222222222222222222222222222222222222222a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7b3333333333333333333333333333333333333333010203040539300000000000001976a914000000000000000000000000000000000000000088aca05b0000000000001976a9140be3e286a15ea85882761618e366586b5574100d88ac00000000".to_string(),
                result: None,
            },
            OpFixture {
                // invalid -- too short 
                txstr: "01000000011111111111111111111111111111111111111111111111111111111111111111000000006a473044022038534377d738ba91df50a4bc885bcd6328520438d42cc29636cc299a24dcb4c202202953e87b6c176697d01d66a742a27fd48b8d2167fb9db184d59a3be23a59992e012102d8015134d9db8178ac93acbc43170a2f20febba5087a5b0437058765ad5133d0000000000300000000000000004c6a4a69645f2222222222222222222222222222222222222222a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a3333333333333333333333333333333333333339300000000000001976a914000000000000000000000000000000000000000088aca05b0000000000001976a9140be3e286a15ea85882761618e366586b5574100d88ac00000000".to_string(),
                result: None,
            },
            OpFixture {
                // invalid -- wrong opcode
                txstr: "01000000011111111111111111111111111111111111111111111111111111111111111111000000006a47304402200e6dbb4ccefc44582135091678a49228716431583dab3d789b1211d5737d02e402205b523ad156cad4ae6bb29f046b144c8c82b7c85698616ee8f5d59ea40d594dd4012102d8015134d9db8178ac93acbc43170a2f20febba5087a5b0437058765ad5133d000000000030000000000000000536a4c5069645e2222222222222222222222222222222222222222a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a3333333333333333333333333333333333333333010203040539300000000000001976a914000000000000000000000000000000000000000088aca05b0000000000001976a9140be3e286a15ea85882761618e366586b5574100d88ac00000000".to_string(),
                result: None,
            }
        ];

        let parser = BitcoinBlockParser::new(BitcoinNetworkType::Testnet, BLOCKSTACK_MAGIC_MAINNET);

        for tx_fixture in tx_fixtures {
            let tx = make_tx(&tx_fixture.txstr).unwrap();
            let burnchain_tx = BurnchainTransaction::Bitcoin(parser.parse_tx(&tx, vtxindex as usize).unwrap());
            
            let header = match tx_fixture.result {
                Some(ref op) => {
                    BurnchainBlockHeader {
                        block_height: op.block_height,
                        block_hash: op.burn_header_hash.clone(),
                        parent_block_hash: op.burn_header_hash.clone(),
                        num_txs: 1,
                        fork_segment_id: op.fork_segment_id,
                        parent_fork_segment_id: op.fork_segment_id,
                        fork_segment_length: 1,
                        fork_length: 1,
                    }
                },
                None => {
                    BurnchainBlockHeader {
                        block_height: 0,
                        block_hash: BurnchainHeaderHash([0u8; 32]),
                        parent_block_hash: BurnchainHeaderHash([0u8; 32]),
                        num_txs: 0,
                        fork_segment_id: 0,
                        parent_fork_segment_id: 0,
                        fork_segment_length: 0,
                        fork_length: 0,
                    }
                }
            };

            let op = UserBurnSupportOp::from_tx(&header, &burnchain_tx);
            
            match (op, tx_fixture.result) {
                (Ok(parsed_tx), Some(result)) => {
                    assert_eq!(parsed_tx, result);
                },
                (Err(_e), None) => {},
                (Ok(_parsed_tx), None) => {
                    test_debug!("Parsed a tx when we should not have");
                    assert!(false);
                },
                (Err(_e), Some(_result)) => {
                    test_debug!("Did not parse a tx when we should have");
                    assert!(false);
                }
            };
        }
    }

    #[test]
    fn test_check() {
        let first_block_height = 120;
        let first_burn_hash = BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000123").unwrap();
        
        let block_122_hash = BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000002").unwrap();
        let block_123_hash = BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000003").unwrap();
        let block_124_hash = BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000004").unwrap();
        let block_125_hash = BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000005").unwrap();
        
        let burnchain = Burnchain {
            peer_version: 0x012345678,
            network_id: 0x9abcdef0,
            chain_name: "bitcoin".to_string(),
            network_name: "testnet".to_string(),
            working_dir: "/nope".to_string(),
            consensus_hash_lifetime: 24,
            stable_confirmations: 7,
            first_block_height: first_block_height,
            first_block_hash: first_burn_hash.clone()
        };
        
        let mut db = BurnDB::connect_memory(first_block_height, &first_burn_hash).unwrap();

        let leader_key_1 = LeaderKeyRegisterOp { 
            consensus_hash: ConsensusHash::from_bytes(&hex_bytes("0000000000000000000000000000000000000000").unwrap()).unwrap(),
            public_key: VRFPublicKey::from_bytes(&hex_bytes("a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a").unwrap()).unwrap(),
            memo: vec![01, 02, 03, 04, 05],
            address: StacksAddress::from_bitcoin_address(&BitcoinAddress::from_scriptpubkey(BitcoinNetworkType::Testnet, &hex_bytes("76a9140be3e286a15ea85882761618e366586b5574100d88ac").unwrap()).unwrap()),

            txid: Txid::from_bytes_be(&hex_bytes("1bfa831b5fc56c858198acb8e77e5863c1e9d8ac26d49ddb914e24d8d4083562").unwrap()).unwrap(),
            vtxindex: 456,
            block_height: 123,
            burn_header_hash: block_123_hash.clone(),

            fork_segment_id: 0
        };
        
        // populate consensus hashes
        {
            let mut tx = db.tx_begin().unwrap();
            let mut prev_snapshot = BurnDB::get_first_block_snapshot(&tx).unwrap(); 
            for i in 0..10 {
                let snapshot_row = BlockSnapshot {
                    block_height: i + 1 + first_block_height,
                    burn_header_hash: BurnchainHeaderHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,i as u8]).unwrap(),
                    parent_burn_header_hash: prev_snapshot.burn_header_hash.clone(),
                    consensus_hash: ConsensusHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,i as u8]).unwrap(),
                    ops_hash: OpsHash::from_bytes(&[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,i as u8]).unwrap(),
                    total_burn: i,
                    sortition: true,
                    sortition_hash: SortitionHash::initial(),
                    winning_block_txid: Txid::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
                    winning_block_burn_hash: BurnchainHeaderHash::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap(),
                    fork_segment_id: 0,
                    parent_fork_segment_id: 0,
                    fork_segment_length: i + 1,
                    fork_length: i + 1,
                };
                BurnDB::append_chain_tip_snapshot(&mut tx, &prev_snapshot, &snapshot_row).unwrap();
                prev_snapshot = snapshot_row;
            }
            
            tx.commit().unwrap();
        }

        {
            let mut tx = db.tx_begin().unwrap();
            BurnDB::insert_leader_key(&mut tx, &leader_key_1).unwrap();
            tx.commit().unwrap();
        }

        let check_fixtures = vec![
            CheckFixture {
                // reject -- bad consensus hash
                op: UserBurnSupportOp {
                    consensus_hash: ConsensusHash::from_bytes(&hex_bytes("1000000000000000000000000000000000000000").unwrap()).unwrap(),
                    public_key: VRFPublicKey::from_bytes(&hex_bytes("a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a").unwrap()).unwrap(),
                    block_header_hash_160: Hash160::from_bytes(&hex_bytes("7150f635054b87df566a970b21e07030d6444bf2").unwrap()).unwrap(),       // 22222....2222
                    key_block_backptr: 1,
                    key_vtxindex: 456,
                    memo: vec![0x05],
                    burn_fee: 10000,

                    txid: Txid::from_bytes_be(&hex_bytes("1d5cbdd276495b07f0e0bf0181fa57c175b217bc35531b078d62fc20986c716b").unwrap()).unwrap(),
                    vtxindex: 13,
                    block_height: 124,
                    burn_header_hash: block_124_hash.clone(),

                    fork_segment_id: 0,
                },
                res: Err(op_error::UserBurnSupportBadConsensusHash),
            },
            CheckFixture {
                // reject -- no leader key
                op: UserBurnSupportOp {
                    consensus_hash: ConsensusHash::from_bytes(&hex_bytes("0000000000000000000000000000000000000000").unwrap()).unwrap(),
                    public_key: VRFPublicKey::from_bytes(&hex_bytes("bb519494643f79f1dea0350e6fb9a1da88dfdb6137117fc2523824a8aa44fe1c").unwrap()).unwrap(),
                    block_header_hash_160: Hash160::from_bytes(&hex_bytes("7150f635054b87df566a970b21e07030d6444bf2").unwrap()).unwrap(),       // 22222....2222
                    key_block_backptr: 1,
                    key_vtxindex: 457,
                    memo: vec![0x05],
                    burn_fee: 10000,

                    txid: Txid::from_bytes_be(&hex_bytes("1d5cbdd276495b07f0e0bf0181fa57c175b217bc35531b078d62fc20986c716b").unwrap()).unwrap(),
                    vtxindex: 13,
                    block_height: 124,
                    burn_header_hash: block_124_hash.clone(),
                    
                    fork_segment_id: 0,
                },
                res: Err(op_error::UserBurnSupportNoLeaderKey),
            },
            CheckFixture {
                // accept 
                op: UserBurnSupportOp {
                    consensus_hash: ConsensusHash::from_bytes(&hex_bytes("0000000000000000000000000000000000000000").unwrap()).unwrap(),
                    public_key: VRFPublicKey::from_bytes(&hex_bytes("a366b51292bef4edd64063d9145c617fec373bceb0758e98cd72becd84d54c7a").unwrap()).unwrap(),
                    block_header_hash_160: Hash160::from_bytes(&hex_bytes("7150f635054b87df566a970b21e07030d6444bf2").unwrap()).unwrap(),       // 22222....2222
                    key_block_backptr: 1,
                    key_vtxindex: 456,
                    memo: vec![0x05],
                    burn_fee: 10000,

                    txid: Txid::from_bytes_be(&hex_bytes("1d5cbdd276495b07f0e0bf0181fa57c175b217bc35531b078d62fc20986c716b").unwrap()).unwrap(),
                    vtxindex: 13,
                    block_height: 124,
                    burn_header_hash: block_124_hash.clone(),
                    
                    fork_segment_id: 0,
                },
                res: Ok(())
            }
        ];

        for fixture in check_fixtures {
            let header = BurnchainBlockHeader {
                block_height: fixture.op.block_height,
                block_hash: fixture.op.burn_header_hash.clone(),
                parent_block_hash: fixture.op.burn_header_hash.clone(),
                num_txs: 1,
                fork_segment_id: fixture.op.fork_segment_id,
                parent_fork_segment_id: fixture.op.fork_segment_id,
                fork_segment_length: 1,
                fork_length: 1,
            };
            let mut tx = db.tx_begin().unwrap();
            assert_eq!(fixture.res, fixture.op.check(&burnchain, &header, &mut tx));
        }
    }
}

