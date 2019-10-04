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
            Error::SocketMutexPoisoned | Error::SocketNotConnectedToPeer => f.write_str(error::Error::description(self)),
            Error::SerializationError(ref e) => fmt::Display::fmt(e, f),
            Error::InvalidMessage(ref _msg) => f.write_str(error::Error::description(self)),
            Error::InvalidReply => f.write_str(error::Error::description(self)),
            Error::InvalidMagic => f.write_str(error::Error::description(self)),
            Error::UnhandledMessage(ref _msg) => f.write_str(error::Error::description(self)),
            Error::ConnectionBroken => f.write_str(error::Error::description(self)),
            Error::ConnectionError => f.write_str(error::Error::description(self)),
            Error::FilesystemError(ref e) => fmt::Display::fmt(e, f),
            Error::HashError(ref e) => fmt::Display::fmt(e, f),
            Error::NoncontiguousHeader => f.write_str(error::Error::description(self)),
            Error::MissingHeader => f.write_str(error::Error::description(self)),
            Error::InvalidPoW => f.write_str(error::Error::description(self)),
            Error::InvalidByteSequence => f.write_str(error::Error::description(self)),
            Error::ConfigError(ref e_str) => fmt::Display::fmt(e_str, f),
            Error::BlockchainHeight => f.write_str(error::Error::description(self)),
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

    fn description(&self) -> &str {
        match *self {
            Error::Io(ref e) => e.description(),
            Error::SocketMutexPoisoned => "socket mutex was poisoned",
            Error::SocketNotConnectedToPeer => "not connected to peer",
            Error::SerializationError(ref e) => e.description(),
            Error::InvalidMessage(ref _msg) => "Invalid message to send",
            Error::InvalidReply => "invalid reply for given message",
            Error::InvalidMagic => "invalid network magic",
            Error::UnhandledMessage(ref _msg) => "Unhandled message",
            Error::ConnectionBroken => "connection to peer node is broken",
            Error::ConnectionError => "connection to peer could not be (re-)established",
            Error::FilesystemError(ref e) => e.description(),
            Error::HashError(ref e) => e.description(),
            Error::NoncontiguousHeader => "Non-contiguous header",
            Error::MissingHeader => "Missing header",
            Error::InvalidPoW => "Invalid proof of work",
            Error::InvalidByteSequence => "Invalid sequence of bytes",
            Error::ConfigError(ref e_str) => e_str.as_str(),
            Error::BlockchainHeight => "Value is beyond the end of the blockchain",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BitcoinNetworkType {
    Mainnet,
    Testnet,
    Regtest
}
