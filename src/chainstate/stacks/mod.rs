/*
 copyright: (c) 2013-2019 by Blockstack PBC, a public benefit corporation.

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

pub mod address;
pub mod auth;
pub mod block;
pub mod index;
pub mod transaction;

use std::ops::Deref;
use std::ops::DerefMut;
use std::fmt;
use std::error;

use util::secp256k1;
use util::hash::Hash160;
use util::vrf::ECVRF_Proof;
use util::hash::Sha512_256;
use util::hash::HASH160_ENCODED_SIZE;

use util::secp256k1::MessageSignature;

use address::AddressHashMode;
use burnchains::Txid;

use chainstate::burn::BlockHeaderHash;

use chainstate::stacks::index::{TrieHash, TRIEHASH_ENCODED_SIZE};

use net::StacksPublicKeyBuffer;
use net::StacksMessageCodec;
use net::codec::{read_next, write_next};
use net::Error as net_error;

pub type StacksPublicKey = secp256k1::Secp256k1PublicKey;
pub type StacksPrivateKey = secp256k1::Secp256k1PrivateKey;

impl_byte_array_message_codec!(TrieHash, TRIEHASH_ENCODED_SIZE as u32);
impl_byte_array_message_codec!(Sha512_256, 32);

pub const C32_ADDRESS_VERSION_MAINNET_SINGLESIG: u8 = 22;       // P
pub const C32_ADDRESS_VERSION_MAINNET_MULTISIG: u8 = 20;        // M
pub const C32_ADDRESS_VERSION_TESTNET_SINGLESIG: u8 = 26;       // T
pub const C32_ADDRESS_VERSION_TESTNET_MULTISIG: u8 = 21;        // N

// "empty" DER-encoded compressed public key bytes.
// Can't be all 0's because the first byte must be 0x02 or 0x03, and the values can't be all 0.
pub const STACKS_PUBLIC_KEY_EMPTY_BYTES : [u8; 33] = [
    0x02,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01
];

impl Txid {
    /// A Stacks transaction ID is a sha512/256 hash (not a double-sha256 hash)
    pub fn from_stacks_tx(txdata: &[u8]) -> Txid {
        let h = Sha512_256::from_data(txdata);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(h.as_bytes());
        Txid(bytes)
    }

    /// A sighash is calculated the same way as a txid
    pub fn from_sighash_bytes(txdata: &[u8]) -> Txid {
        Txid::from_stacks_tx(txdata)
    }
}

#[derive(Debug)]
pub enum Error {
    /// Failed to encode
    EncodeError,
    /// Failed to decode 
    DecodeError,
    /// Failed to validate spending condition 
    AuthError,
    /// Invalid transaction fee
    InvalidFee
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::EncodeError => f.write_str(error::Error::description(self)),
            Error::DecodeError => f.write_str(error::Error::description(self)),
            Error::AuthError => f.write_str(error::Error::description(self)),
            Error::InvalidFee => f.write_str(error::Error::description(self)),
        }
    }
}

impl error::Error for Error {
    fn cause(&self) -> Option<&error::Error> {
        match *self {
            Error::EncodeError => None,
            Error::DecodeError => None,
            Error::AuthError => None,
            Error::InvalidFee => None,
        }
    }

    fn description(&self) -> &str {
        match *self {
            Error::EncodeError => "Failed to encode",
            Error::DecodeError => "Failed to decode",
            Error::AuthError => "Failed to authenticate transaction",
            Error::InvalidFee => "Invalid transaction fee",
        }
    }
}

/// printable-ASCII-only string, but encodable.
/// Note that it cannot be longer than ARRAY_MAX_LEN (4.1 billion bytes)
#[derive(Clone, PartialEq)]
pub struct StacksString(Vec<u8>);

impl fmt::Display for StacksString {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(String::from_utf8_lossy(&self).into_owned().as_str())
    }
}

impl fmt::Debug for StacksString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(String::from_utf8_lossy(&self).into_owned().as_str())
    }
}

impl Deref for StacksString {
    type Target = Vec<u8>;
    fn deref(&self) -> &Vec<u8> {
        &self.0
    }
}

impl DerefMut for StacksString {
    fn deref_mut(&mut self) -> &mut Vec<u8> {
        &mut self.0
    }
}

impl StacksMessageCodec for StacksString {
    fn serialize(&self) -> Vec<u8> {
        let mut res = vec![];
        write_next(&mut res, &self.0);
        res
    }

    fn deserialize(buf: &Vec<u8>, index: &mut u32, max_size: u32) -> Result<StacksString, net_error> {
        let bytes : Vec<u8> = read_next(buf, index, max_size)?;

        // must encode a valid string
        let s = String::from_utf8(bytes.clone())
            .map_err(|_e| net_error::DeserializeError)?;
        
        if !StacksString::is_valid_string(&s) {
            // non-printable ASCII or not ASCII
            return Err(net_error::DeserializeError);
        }

        Ok(StacksString(bytes))
    }
}

impl StacksString {
    /// Is the given string a valid Clarity string?
    pub fn is_valid_string(s: &String) -> bool {
        s.is_ascii() && StacksString::is_printable(s)
    }

    /// Is the given string a well-formed name for a Clarity smart contract?
    pub fn is_valid_contract_name(s: &String) -> bool {
        StacksString::is_valid_string(s) && s.find('.').is_none()
    }
    
    /// Is the given string a well-formed name for a Clarity asset?
    pub fn is_valid_asset_name(s: &String) -> bool {
        // TODO: verify that we don't want periods in asset names
        StacksString::is_valid_string(s) && s.find('.').is_none()
    }

    /// Is the given string a well-formed name for a non-fungible token?
    pub fn is_valid_nft_name(s: &String) -> bool {
        // TODO: verify that this is sufficient
        StacksString::is_valid_string(s)
    }

    pub fn is_printable(s: &String) -> bool {
        if !s.is_ascii() {
            return false;
        }
        // all characters must be ASCII "printable" characters, excluding "delete".
        // This is 0x20 through 0x7e, inclusive
        for c in s.as_bytes().iter() {
            if (*c as u8) < 0x20 || (*c as u8) > 0x7e {
                return false;
            }
        }
        true
    }

    pub fn from_string(s: &String) -> Option<StacksString> {
        if !StacksString::is_valid_string(s) {
            return None;
        }
        Some(StacksString(s.as_bytes().to_vec()))
    }

    pub fn from_contract_name(s: &String) -> Option<StacksString> {
        if !StacksString::is_valid_contract_name(s) {
            return None;
        }
        Some(StacksString(s.as_bytes().to_vec()))
    }

    pub fn from_asset_name(s: &String) -> Option<StacksString> {
        if !StacksString::is_valid_asset_name(s) {
            return None;
        }
        Some(StacksString(s.as_bytes().to_vec()))
    }

    pub fn from_nft_name(s: &String) -> Option<StacksString> {
        if !StacksString::is_valid_nft_name(s) {
            return None;
        }
        Some(StacksString(s.as_bytes().to_vec()))
    }

    pub fn from_str(s: &str) -> Option<StacksString> {
        if !StacksString::is_valid_string(&String::from(s)) {
            return None;
        }
        Some(StacksString(s.as_bytes().to_vec()))
    }

    pub fn to_string(&self) -> String {
        // guaranteed to always succeed because the string is ASCII
        String::from_utf8(self.0.clone()).unwrap()
    }
} 

#[derive(Debug, Clone, PartialEq, Copy)]
pub struct StacksAddress {
    pub version: u8,
    pub bytes: Hash160
}

pub const STACKS_ADDRESS_ENCODED_SIZE : u32 = 1 + HASH160_ENCODED_SIZE;

/// How a transaction may be appended to the Stacks blockchain
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum TransactionAnchorMode {
    OnChainOnly = 1,        // must be included in a StacksBlock
    OffChainOnly = 2,       // must be included in a StacksMicroBlock
    Any = 3                 // either
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum TransactionAuthFlags {
    // types of auth
    AuthStandard = 0x04,
    AuthSponsored = 0x05,
}

/// Transaction signatures are validated by calculating the public key from the signature, and
/// verifying that all public keys hash to the signing account's hash.  To do so, we must preserve
/// enough information in the auth structure to recover each public key's bytes.
/// 
/// An auth field can be a public key or a signature.  In both cases, the public key (either given
/// in-the-raw or embedded in a signature) may be encoded as compressed or uncompressed.
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum TransactionAuthFieldID {
    // types of auth fields
    PublicKeyCompressed = 0x00,
    PublicKeyUncompressed = 0x01,
    SignatureCompressed = 0x02,
    SignatureUncompressed = 0x03
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum TransactionPublicKeyEncoding {
    // ways we can encode a public key
    Compressed = 0x00,
    Uncompressed = 0x01
}

impl TransactionPublicKeyEncoding {
    pub fn from_u8(n: u8) -> Option<TransactionPublicKeyEncoding> {
        match n {
            x if x == TransactionPublicKeyEncoding::Compressed as u8 => Some(TransactionPublicKeyEncoding::Compressed),
            x if x == TransactionPublicKeyEncoding::Uncompressed as u8 => Some(TransactionPublicKeyEncoding::Uncompressed),
            _ => None
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransactionAuthField {
    PublicKey(StacksPublicKey),
    Signature(TransactionPublicKeyEncoding, MessageSignature)
}

impl TransactionAuthField {
    pub fn is_public_key(&self) -> bool {
        match *self {
            TransactionAuthField::PublicKey(_) => true,
            _ => false
        }
    }
    
    pub fn is_signature(&self) -> bool {
        match *self {
            TransactionAuthField::Signature(_, _) => true,
            _ => false
        }
    }

    pub fn as_public_key(&self) -> Option<StacksPublicKey> {
        match *self {
            TransactionAuthField::PublicKey(ref pubk) => Some(pubk.clone()),
            _ => None
        }
    }

    pub fn as_signature(&self) -> Option<(TransactionPublicKeyEncoding, MessageSignature)> {
        match *self {
            TransactionAuthField::Signature(ref key_fmt, ref sig) => Some((key_fmt.clone(), sig.clone())),
            _ => None
        }
    }

    pub fn get_public_key(&self, sighash_bytes: &[u8]) -> Result<StacksPublicKey, net_error> {
        match *self {
            TransactionAuthField::PublicKey(ref pubk) => Ok(pubk.clone()),
            TransactionAuthField::Signature(ref key_fmt, ref sig) => {
                let mut pubk = StacksPublicKey::recover_to_pubkey(sighash_bytes, sig).map_err(|e| net_error::VerifyingError(e.to_string()))?;
                pubk.set_compressed(if *key_fmt == TransactionPublicKeyEncoding::Compressed { true } else { false });
                Ok(pubk)
            }
        }
    }
}

// tag address hash modes as "singlesig" or "multisig" so we can't accidentally construct an
// invalid spending condition
#[repr(u8)]
#[derive(Debug, Clone, PartialEq)]
pub enum SinglesigHashMode {
    P2PKH = 0x00,
    P2WPKH = 0x02,
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq)]
pub enum MultisigHashMode {
    P2SH = 0x01,
    P2WSH = 0x03
}

impl SinglesigHashMode {
    pub fn to_address_hash_mode(&self) -> AddressHashMode {
        match *self {
            SinglesigHashMode::P2PKH => AddressHashMode::SerializeP2PKH,
            SinglesigHashMode::P2WPKH => AddressHashMode::SerializeP2WPKH
        }
    }

    pub fn from_address_hash_mode(hm: AddressHashMode) -> Option<SinglesigHashMode> {
        match hm {
            AddressHashMode::SerializeP2PKH => Some(SinglesigHashMode::P2PKH),
            AddressHashMode::SerializeP2WPKH => Some(SinglesigHashMode::P2WPKH),
            _ => None
        }
    }

    pub fn from_u8(n: u8) -> Option<SinglesigHashMode> {
        match n {
            x if x == SinglesigHashMode::P2PKH as u8 => Some(SinglesigHashMode::P2PKH),
            x if x == SinglesigHashMode::P2WPKH as u8 => Some(SinglesigHashMode::P2WPKH),
            _ => None
        }
    }
}

impl MultisigHashMode {
    pub fn to_address_hash_mode(&self) -> AddressHashMode {
        match *self {
            MultisigHashMode::P2SH => AddressHashMode::SerializeP2SH,
            MultisigHashMode::P2WSH => AddressHashMode::SerializeP2WSH
        }
    }

    pub fn from_address_hash_mode(hm: AddressHashMode) -> Option<MultisigHashMode> {
        match hm {
            AddressHashMode::SerializeP2SH => Some(MultisigHashMode::P2SH),
            AddressHashMode::SerializeP2WSH => Some(MultisigHashMode::P2WSH),
            _ => None
        }
    }
    
    pub fn from_u8(n: u8) -> Option<MultisigHashMode> {
        match n {
            x if x == MultisigHashMode::P2SH as u8 => Some(MultisigHashMode::P2SH),
            x if x == MultisigHashMode::P2WSH as u8 => Some(MultisigHashMode::P2WSH),
            _ => None
        }
    }
}

/// A structure that encodes enough state to authenticate
/// a transaction's execution against a Stacks address.
/// public_keys + signatures_required determines the Principal.
/// nonce is the "check number" for the Principal.
#[derive(Debug, Clone, PartialEq)]
pub struct MultisigSpendingCondition {
    pub hash_mode: MultisigHashMode,
    pub signer: Hash160,
    pub nonce: u64,                             // nth authorization from this account
    pub fields: Vec<TransactionAuthField>,
    pub signatures_required: u16
}

#[derive(Debug, Clone, PartialEq)]
pub struct SinglesigSpendingCondition {
    pub hash_mode: SinglesigHashMode,
    pub signer: Hash160,
    pub nonce: u64,                             // nth authorization from this account
    pub key_encoding: TransactionPublicKeyEncoding,
    pub signature: MessageSignature
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransactionSpendingCondition {
    Singlesig(SinglesigSpendingCondition),
    Multisig(MultisigSpendingCondition)
}

/// Types of transaction authorizations
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionAuth {
    Standard(TransactionSpendingCondition),
    Sponsored(TransactionSpendingCondition, TransactionSpendingCondition),  // the second account pays on behalf of the first account
}

/// A transaction that calls into a smart contract
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionContractCall {
    pub contract_call: StacksString
}

/// A transaction that instantiates a smart contract
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionSmartContract {
    pub name: StacksString,
    pub code_body: StacksString
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransactionPayload {
    ContractCall(TransactionContractCall),
    SmartContract(TransactionSmartContract),
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum TransactionPayloadID {
    SmartContract = 0,
    ContractCall = 1,
}

/// Encoding of an asset type identifier 
#[derive(Debug, Clone, PartialEq)]
pub struct AssetInfo {
    pub contract_address: StacksAddress,
    pub asset_name: StacksString,
}

/// type of asset
#[derive(Debug, Clone, PartialEq)]
pub enum AssetType {
    STX,
    FungibleAsset(AssetInfo),
    NonfungibleAsset(AssetInfo, StacksString)
}

/// numeric wire-format ID of an asset type
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum AssetTypeID {
    STX = 0,
    FungibleAsset = 1,
    NonfungibleAsset = 2
}

impl AssetTypeID {
    pub fn from_u8(b: u8) -> Option<AssetTypeID> {
        match b {
            0 => Some(AssetTypeID::STX),
            1 => Some(AssetTypeID::FungibleAsset),
            2 => Some(AssetTypeID::NonfungibleAsset),
            _ => None
        }
    }
}

/// Encoding of a transaction fee. Could be in something besides STX.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionFee {
    pub asset: AssetType,
    pub amount: u64,
    pub exchange_rate: u64      // how many microSTX is this asset worth
}

impl TransactionFee {
    pub fn to_microstx(amount: u64, exchange_rate: u64) -> Result<u64, Error> {
        amount.checked_mul(exchange_rate)
            .ok_or(Error::InvalidFee)
    }

    pub fn as_microstx(&self) -> Result<u64, Error> {
        TransactionFee::to_microstx(self.amount, self.exchange_rate)
    }
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum FungibleConditionCode {
    NoChange = 0x00,
    IncEq = 0x01,
    IncGt = 0x02,
    IncGe = 0x03,
    IncLt = 0x04,
    IncLe = 0x05,
    DecEq = 0x81,
    DecGt = 0x82,
    DecGe = 0x83,
    DecLt = 0x84,
    DecLe = 0x85,
}

impl FungibleConditionCode {
    pub fn from_u8(b: u8) -> Option<FungibleConditionCode> {
        match b {
            0x00 => Some(FungibleConditionCode::NoChange),
            0x01 => Some(FungibleConditionCode::IncEq),
            0x02 => Some(FungibleConditionCode::IncGt),
            0x03 => Some(FungibleConditionCode::IncGe),
            0x04 => Some(FungibleConditionCode::IncLt),
            0x05 => Some(FungibleConditionCode::IncLe),
            0x81 => Some(FungibleConditionCode::DecEq),
            0x82 => Some(FungibleConditionCode::DecGt),
            0x83 => Some(FungibleConditionCode::DecGe),
            0x84 => Some(FungibleConditionCode::DecLt),
            0x85 => Some(FungibleConditionCode::DecLe),
            _ => None
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum NonfungibleConditionCode {
    NoChange = 0x00,
    Absent = 0x10,
    Present = 0x11
}

impl NonfungibleConditionCode {
    pub fn from_u8(b: u8) -> Option<NonfungibleConditionCode> {
        match b {
            0x00 => Some(NonfungibleConditionCode::NoChange),
            0x10 => Some(NonfungibleConditionCode::Absent),
            0x11 => Some(NonfungibleConditionCode::Present),
            _ => None
        }
    }
}

/// Post-condition on a transaction
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionPostCondition {
    STX(FungibleConditionCode, u64),
    Fungible(AssetType, FungibleConditionCode, u64),
    Nonfungible(AssetType, NonfungibleConditionCode)
}

/// Stacks transaction versions
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum TransactionVersion {
    Mainnet = 0x00,
    Testnet = 0x80
}

#[derive(Debug, Clone, PartialEq)]
pub struct StacksTransaction {
    pub version: TransactionVersion,
    pub chain_id: u32,
    pub auth: TransactionAuth,
    pub fee: TransactionFee,
    pub anchor_mode: TransactionAnchorMode,
    pub post_conditions: Vec<TransactionPostCondition>,
    pub payload: TransactionPayload
}

#[derive(Debug, Clone, PartialEq)]
pub struct StacksTransactionSigner {
    pub tx: StacksTransaction,
    pub sighash: Txid,
    origin_done: bool
}

/// How much work has gone into this chain so far?
#[derive(Debug, Clone, PartialEq)]
pub struct StacksWorkScore {
    pub burn: u64,
    pub work: u64
}

/// The header for an on-chain-anchored Stacks block
#[derive(Debug, Clone, PartialEq)]
pub struct StacksBlockHeader {
    version: u8,
    total_work: StacksWorkScore,
    proof: ECVRF_Proof,
    parent_block: BlockHeaderHash,
    parent_microblock: BlockHeaderHash,
    tx_merkle_root: Sha512_256,
    state_index_root: TrieHash,
    microblock_pubkey: StacksPublicKey
}

/// A block that contains blockchain-anchored data 
/// (corresponding to a LeaderBlockCommitOp)
#[derive(Debug, Clone, PartialEq)]
pub struct StacksBlock {
    header: StacksBlockHeader,
    txs: Vec<StacksTransaction>
}

/// Header structure for a microblock
#[derive(Debug, Clone, PartialEq)]
pub struct StacksMicroblockHeader {
    version: u8,
    sequence: u32,
    prev_block: BlockHeaderHash,
    tx_merkle_root: Sha512_256,
    signature: MessageSignature
}

/// A microblock that contains non-blockchain-anchored data,
/// but is tied to an on-chain block 
#[derive(Debug, Clone, PartialEq)]
pub struct StacksMicroblock {
    header: StacksMicroblockHeader,
    txs: Vec<StacksTransaction>
}

// maximum block size is 1MB.  Complaints to /dev/null -- if you need bigger, start an app chain
pub const MAX_BLOCK_SIZE : u32 = 1048576;

// maximum microblock size is 64KB
pub const MAX_MICROBLOCK_SIZE : u32 = 65536;

// maximum microblocks between stacks blocks (amounts to 16MB of data at max)
pub const MAX_MICROBLOCK_SEQUENCE_LEN : u32 = 256;
