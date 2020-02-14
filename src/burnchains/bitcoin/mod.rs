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

// This module is concerned with the implementation of the BitcoinIndexer
// structure and its methods and traits.

pub mod address;
pub mod bits;
pub mod blocks;
pub mod messages;
pub mod keys;
pub mod indexer;
pub mod network;
pub mod spv;

use std::fmt;
use std::io;
use std::error;
use std::sync::Arc;

use chainstate::burn::operations::BlockstackOperationType;

use burnchains::bitcoin::address::BitcoinAddress;
use burnchains::bitcoin::keys::BitcoinPublicKey;
use burnchains::{
    BurnchainHeaderHash,
    Txid
};

use deps;

use deps::bitcoin::network::serialize::Error as btc_serialize_error;

use util::HexError as btc_hex_error;

pub type PeerMessage = Arc<deps::bitcoin::network::message::NetworkMessage>;

// Borrowed from Andrew Poelstra's rust-bitcoin 

/// Network error
#[derive(Debug)]
pub enum Error {
    /// I/O error
    Io(io::Error),
    /// Socket mutex was poisoned
    SocketMutexPoisoned,
    /// Not connected to peer
    SocketNotConnectedToPeer,
    /// Serialization error 
    SerializationError(btc_serialize_error),
    /// Invalid Message to peer
    InvalidMessage(PeerMessage),
    /// Invalid Reply from peer
    InvalidReply,
    /// Invalid magic 
    InvalidMagic,
    /// Unhandled message 
    UnhandledMessage(PeerMessage),
    /// Connection is broken and ought to be re-established
    ConnectionBroken,
    /// Connection could not be (re-)established
    ConnectionError,
    /// general filesystem error
    FilesystemError(io::Error),
    /// Hashing error
    HashError(btc_hex_error),
    /// Non-contiguous header 
    NoncontiguousHeader,
    /// Missing header
    MissingHeader,
    /// Invalid target 
    InvalidPoW,
    /// Wrong number of bytes for constructing an address
    InvalidByteSequence,
    /// Configuration error 
    ConfigError(String),
    /// Tried to synchronize to a point above the chain tip
    BlockchainHeight,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::Io(ref e) => fmt::Display::fmt(e, f),
            Error::SocketMutexPoisoned => write!(f, "socket mutex was poisoned"),
            Error::SocketNotConnectedToPeer => write!(f, "not connected to peer"),
            Error::SerializationError(ref e) => fmt::Display::fmt(e, f),
            Error::InvalidMessage(ref _msg) => write!(f, "Invalid message to send"),
            Error::InvalidReply => write!(f, "invalid reply for given message"),
            Error::InvalidMagic => write!(f, "invalid network magic"),
            Error::UnhandledMessage(ref _msg) => write!(f, "Unhandled message"),
            Error::ConnectionBroken => write!(f, "connection to peer node is broken"),
            Error::ConnectionError => write!(f, "connection to peer could not be (re-)established"),
            Error::FilesystemError(ref e) => fmt::Display::fmt(e, f),
            Error::HashError(ref e) => fmt::Display::fmt(e, f),
            Error::NoncontiguousHeader => write!(f, "Non-contiguous header"),
            Error::MissingHeader => write!(f, "Missing header"),
            Error::InvalidPoW => write!(f, "Invalid proof of work"),
            Error::InvalidByteSequence => write!(f, "Invalid sequence of bytes"),
            Error::ConfigError(ref e_str) => fmt::Display::fmt(e_str, f),
            Error::BlockchainHeight => write!(f, "Value is beyond the end of the blockchain"),
        }
    }
}

impl error::Error for Error {
    fn cause(&self) -> Option<&dyn error::Error> {
        match *self {
            Error::Io(ref e) => Some(e),
            Error::SocketMutexPoisoned | Error::SocketNotConnectedToPeer => None,
            Error::SerializationError(ref e) => Some(e),
            Error::InvalidMessage(ref _msg) => None,
            Error::InvalidReply => None,
            Error::InvalidMagic => None,
            Error::UnhandledMessage(ref _msg) => None,
            Error::ConnectionBroken => None,
            Error::ConnectionError => None,
            Error::FilesystemError(ref e) => Some(e),
            Error::HashError(ref e) => Some(e),
            Error::NoncontiguousHeader => None,
            Error::MissingHeader => None,
            Error::InvalidPoW => None,
            Error::InvalidByteSequence => None,
            Error::ConfigError(ref _e_str) => None,
            Error::BlockchainHeight => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BitcoinNetworkType {
    Mainnet,
    Testnet,
    Regtest
}

#[derive(Debug, PartialEq, Clone, Eq, Serialize, Deserialize)]
pub struct BitcoinTxOutput {
    pub address: BitcoinAddress,
    pub units: u64
}

#[derive(Debug, PartialEq, Clone, Eq, Serialize, Deserialize)]
pub enum BitcoinInputType {
    Standard,
    SegwitP2SH
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct BitcoinTxInput {
    pub keys: Vec<BitcoinPublicKey>,
    pub num_required: usize,
    pub in_type: BitcoinInputType
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BitcoinTransaction {
    pub txid: Txid,
    pub vtxindex: u32,
    pub opcode: u8,
    pub data: Vec<u8>,
    pub inputs: Vec<BitcoinTxInput>,
    pub outputs: Vec<BitcoinTxOutput>
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct BitcoinBlock {
    pub block_height: u64,
    pub block_hash: BurnchainHeaderHash,
    pub parent_block_hash: BurnchainHeaderHash,
    pub txs: Vec<BitcoinTransaction>,
    pub timestamp: u64
}

impl BitcoinBlock {
    pub fn new(height: u64, hash: &BurnchainHeaderHash, parent: &BurnchainHeaderHash, txs: &Vec<BitcoinTransaction>, timestamp: u64) -> BitcoinBlock {
        BitcoinBlock {
            block_height: height,
            block_hash: hash.clone(),
            parent_block_hash: parent.clone(),
            txs: txs.clone(),
            timestamp: timestamp
        }
    }
}
