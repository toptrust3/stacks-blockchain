use std::fmt;
use std::io::Write;
use std::str::FromStr;

use curve25519_dalek::digest::Digest;
use sha2::Sha512Trunc256;

use util::hash::{Hash160, HASH160_ENCODED_SIZE, Sha512Trunc256Sum, to_hex};
use util::secp256k1::MessageSignature;
use util::vrf::VRFProof;

#[derive(Serialize, Deserialize)]
pub struct BurnchainHeaderHash(pub [u8; 32]);
impl_array_newtype!(BurnchainHeaderHash, u8, 32);
impl_array_hexstring_fmt!(BurnchainHeaderHash);
impl_byte_array_newtype!(BurnchainHeaderHash, u8, 32);

pub struct BlockHeaderHash(pub [u8; 32]);
impl_array_newtype!(BlockHeaderHash, u8, 32);
impl_array_hexstring_fmt!(BlockHeaderHash);
impl_byte_array_newtype!(BlockHeaderHash, u8, 32);
impl_byte_array_serde!(BlockHeaderHash);
pub const BLOCK_HEADER_HASH_ENCODED_SIZE: usize = 32;

/// Identifier used to identify "sortitions" in the
///  SortitionDB. A sortition is the collection of
///  valid burnchain operations (and any dependent
///  variables, e.g., the sortition winner, the
///  consensus hash, the next VRF key)
pub struct SortitionId(pub [u8; 32]);
impl_array_newtype!(SortitionId, u8, 32);
impl_array_hexstring_fmt!(SortitionId);
impl_byte_array_newtype!(SortitionId, u8, 32);
impl_byte_array_from_column!(SortitionId);
impl_byte_array_message_codec!(SortitionId, 32);

pub struct VRFSeed(pub [u8; 32]);
impl_array_newtype!(VRFSeed, u8, 32);
impl_array_hexstring_fmt!(VRFSeed);
impl_byte_array_newtype!(VRFSeed, u8, 32);
impl_byte_array_serde!(VRFSeed);
pub const VRF_SEED_ENCODED_SIZE: u32 = 32;

/// Identifier used to identify Proof-of-Transfer forks
///  (or Rewards Cycle forks). These identifiers are opaque
///  outside of the PoX DB, however, they are sufficient
///  to uniquely identify a "sortition" when paired with
///  a burn header hash
// TODO: Vec<bool> is an aggressively unoptimized implementation,
//       replace with a real bitvec
#[derive(Clone, Debug, PartialEq)]
pub struct PoxId(Vec<bool>);

impl PoxId {
    pub fn new(contents: Vec<bool>) -> Self {
        PoxId(contents)
    }

    pub fn initial() -> PoxId {
        PoxId(vec![true])
    }

    pub fn from_bools(bools: Vec<bool>) -> PoxId {
        PoxId(bools)
    }

    pub fn extend_with_present_block(&mut self) {
        self.0.push(true);
    }
    pub fn extend_with_not_present_block(&mut self) {
        self.0.push(false);
    }

    pub fn stubbed() -> PoxId {
        PoxId(vec![])
    }

    pub fn has_ith_anchor_block(&self, i: usize) -> bool {
        if i >= self.0.len() {
            false
        } else {
            self.0[i]
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn bit_slice(&self, start: usize, len: usize) -> (Vec<u8>, u64) {
        let mut ret = vec![0x00];
        let mut count = 0;
        for bit in start..(start + len) {
            if bit >= self.len() {
                break;
            }
            let i = bit - start;
            if i > 0 && i % 8 == 0 {
                ret.push(0x00);
            }

            let sz = ret.len() - 1;
            if self.0[bit] {
                ret[sz] |= 1 << (i % 8);
            }
            count += 1;
        }
        (ret, count)
    }

    pub fn num_inventory_reward_cycles(&self) -> usize {
        self.0.len().saturating_sub(1)
    }
}

impl fmt::Display for PoxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for val in self.0.iter() {
            write!(f, "{}", if *val { 1 } else { 0 })?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Copy, Serialize, Deserialize, Hash)]
pub struct StacksAddress {
    pub version: u8,
    pub bytes: Hash160,
}

pub const STACKS_ADDRESS_ENCODED_SIZE: u32 = 1 + HASH160_ENCODED_SIZE;

/// How much work has gone into this chain so far?
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StacksWorkScore {
    pub burn: u64, // number of burn tokens destroyed
    pub work: u64, // in Stacks, "work" == the length of the fork
}

/// Hash of a Trie node.  This is a SHA2-512/256.
pub struct TrieHash(pub [u8; 32]);
impl_array_newtype!(TrieHash, u8, 32);
impl_array_hexstring_fmt!(TrieHash);
impl_byte_array_newtype!(TrieHash, u8, 32);
impl_byte_array_serde!(TrieHash);
pub const TRIEHASH_ENCODED_SIZE: usize = 32;

/// The header for an on-chain-anchored Stacks block
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StacksBlockHeader {
    pub version: u8,
    pub total_work: StacksWorkScore, // NOTE: this is the work done on the chain tip this block builds on (i.e. take this from the parent)
    pub proof: VRFProof,
    pub parent_block: BlockHeaderHash, // NOTE: even though this is also present in the burn chain, we need this here for super-light clients that don't even have burn chain headers
    pub parent_microblock: BlockHeaderHash,
    pub parent_microblock_sequence: u16,
    pub tx_merkle_root: Sha512Trunc256Sum,
    pub state_index_root: TrieHash,
    pub microblock_pubkey_hash: Hash160, // we'll get the public key back from the first signature (note that this is the Hash160 of the _compressed_ public key)
}

pub struct StacksBlockId(pub [u8; 32]);
impl_array_newtype!(StacksBlockId, u8, 32);
impl_array_hexstring_fmt!(StacksBlockId);
impl_byte_array_newtype!(StacksBlockId, u8, 32);
impl_byte_array_from_column!(StacksBlockId);
impl_byte_array_serde!(StacksBlockId);

/// Header structure for a microblock
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StacksMicroblockHeader {
    pub version: u8,
    pub sequence: u16,
    pub prev_block: BlockHeaderHash,
    pub tx_merkle_root: Sha512Trunc256Sum,
    pub signature: MessageSignature,
}

#[derive(Debug)]
pub struct TrieMerkleProof<T: ClarityMarfTrieId>(pub Vec<TrieMerkleProofType<T>>);

pub trait ClarityMarfTrieId:
PartialEq
    + Clone
    + std::fmt::Display
    + std::fmt::Debug
    + std::convert::From<[u8; 32]>
{
    fn as_bytes(&self) -> &[u8];
    fn to_bytes(self) -> [u8; 32];
    fn from_bytes([u8; 32]) -> Self;
    fn sentinel() -> Self;
}

#[derive(Clone)]
pub enum TrieMerkleProofType<T> {
    Node4((u8, ProofTrieNode<T>, [TrieHash; 3])),
    Node16((u8, ProofTrieNode<T>, [TrieHash; 15])),
    Node48((u8, ProofTrieNode<T>, [TrieHash; 47])),
    Node256((u8, ProofTrieNode<T>, [TrieHash; 255])),
    Leaf((u8, TrieLeaf)),
    Shunt((i64, Vec<TrieHash>)),
}

pub fn hashes_fmt(hashes: &[TrieHash]) -> String {
    let mut strs = vec![];
    if hashes.len() < 48 {
        for i in 0..hashes.len() {
            strs.push(format!("{:?}", hashes[i]));
        }
        strs.join(",")
    } else {
        for i in 0..hashes.len() / 4 {
            strs.push(format!(
                "{:?},{:?},{:?},{:?}",
                hashes[4 * i],
                hashes[4 * i + 1],
                hashes[4 * i + 2],
                hashes[4 * i + 3]
            ));
        }
        format!("\n{}", strs.join("\n"))
    }
}

impl<T: ClarityMarfTrieId> fmt::Debug for TrieMerkleProofType<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TrieMerkleProofType::Node4((ref chr, ref node, ref hashes)) => write!(
                f,
                "TrieMerkleProofType::Node4(0x{:02x}, node={:?}, hashes={})",
                chr,
                node,
                hashes_fmt(hashes)
            ),
            TrieMerkleProofType::Node16((ref chr, ref node, ref hashes)) => write!(
                f,
                "TrieMerkleProofType::Node16(0x{:02x}, node={:?}, hashes={})",
                chr,
                node,
                hashes_fmt(hashes)
            ),
            TrieMerkleProofType::Node48((ref chr, ref node, ref hashes)) => write!(
                f,
                "TrieMerkleProofType::Node48(0x{:02x}, node={:?}, hashes={})",
                chr,
                node,
                hashes_fmt(hashes)
            ),
            TrieMerkleProofType::Node256((ref chr, ref node, ref hashes)) => write!(
                f,
                "TrieMerkleProofType::Node256(0x{:02x}, node={:?}, hashes={})",
                chr,
                node,
                hashes_fmt(hashes)
            ),
            TrieMerkleProofType::Leaf((ref chr, ref node)) => write!(
                f,
                "TrieMerkleProofType::Leaf(0x{:02x}, node={:?})",
                chr, node
            ),
            TrieMerkleProofType::Shunt((ref idx, ref hashes)) => write!(
                f,
                "TrieMerkleProofType::Shunt(idx={}, hashes={:?})",
                idx, hashes
            ),
        }
    }
}

/// Merkle Proof Trie Pointers have a different structure
///   than the runtime representation --- the proof includes
///   the block header hash for back pointers.
#[derive(Debug, Clone, PartialEq)]
pub struct ProofTrieNode<T> {
    pub id: u8,
    pub path: Vec<u8>,
    pub ptrs: Vec<ProofTriePtr<T>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProofTriePtr<T> {
    pub id: u8,
    pub chr: u8,
    pub back_block: T,
}

/// Structure that holds the actual data in a MARF leaf node.
/// It only stores the hash of some value string, but we add 8 extra bytes for future extensions.
/// If not used (the rule today), then they should all be 0.
pub struct MARFValue(pub [u8; 40]);
impl_array_newtype!(MARFValue, u8, 40);
impl_array_hexstring_fmt!(MARFValue);
impl_byte_array_newtype!(MARFValue, u8, 40);
impl_byte_array_message_codec!(MARFValue, 40);
pub const MARF_VALUE_ENCODED_SIZE: u32 = 40;

impl From<u32> for MARFValue {
    fn from(value: u32) -> MARFValue {
        let h = value.to_le_bytes();
        let mut d = [0u8; MARF_VALUE_ENCODED_SIZE as usize];
        if h.len() > MARF_VALUE_ENCODED_SIZE as usize {
            panic!("Cannot convert a u32 into a MARF Value.");
        }
        for i in 0..h.len() {
            d[i] = h[i];
        }
        MARFValue(d)
    }
}

/// Leaf of a Trie.
#[derive(Clone)]
pub struct TrieLeaf {
    pub path: Vec<u8>,   // path to be lazily expanded
    pub data: MARFValue, // the actual data
}

impl fmt::Debug for TrieLeaf {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "TrieLeaf(path={} data={})",
            &to_hex(&self.path),
            &to_hex(&self.data.to_vec())
        )
    }
}

