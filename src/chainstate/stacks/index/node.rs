// Copyright (C) 2013-2020 Blockstack PBC, a public benefit corporation
// Copyright (C) 2020 Stacks Open Internet Foundation
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::char::from_digit;
use std::collections::{HashMap, HashSet, VecDeque};
use std::error;
use std::fmt;
use std::io;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use sha2::Digest;

use chainstate::stacks::index::bits::{
    get_path_byte_len, get_ptrs_byte_len, path_from_bytes, ptrs_from_bytes, write_path_to_bytes,
};
use chainstate::stacks::index::Error;
use chainstate::stacks::index::{slice_partialeq, BlockMap, MarfTrieId, TrieHasher};
use net::{codec::read_next, StacksMessageCodec};
use util::hash::to_hex;
use util::log;

use crate::types::chainstate::{
    BlockHeaderHash, ClarityMarfTrieId, MARFValue, TrieLeaf, MARF_VALUE_ENCODED_SIZE,
};
use crate::types::chainstate::{TrieHash, BLOCK_HEADER_HASH_ENCODED_SIZE, TRIEHASH_ENCODED_SIZE};

#[derive(Debug, Clone, PartialEq)]
pub enum CursorError {
    PathDiverged,
    BackptrEncountered(TriePtr),
    ChrNotFound,
}

impl fmt::Display for CursorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CursorError::PathDiverged => write!(f, "Path diverged"),
            CursorError::BackptrEncountered(_) => write!(f, "Back-pointer encountered"),
            CursorError::ChrNotFound => write!(f, "Node child not found"),
        }
    }
}

impl error::Error for CursorError {
    fn cause(&self) -> Option<&dyn error::Error> {
        None
    }
}

// All numeric values of a Trie node when encoded.
// They are all 7-bit numbers -- the 8th bit is used to indicate whether or not the value
// identifies a back-pointer to be followed.
define_u8_enum!(TrieNodeID {
    Empty = 0,
    Leaf = 1,
    Node4 = 2,
    Node16 = 3,
    Node48 = 4,
    Node256 = 5
});

/// A node ID encodes a back-pointer if its high bit is set
pub fn is_backptr(id: u8) -> bool {
    id & 0x80 != 0
}

/// Set the back-pointer bit
pub fn set_backptr(id: u8) -> u8 {
    id | 0x80
}

/// Clear the back-pointer bit
pub fn clear_backptr(id: u8) -> u8 {
    id & 0x7f
}

// Byte writing operations for pointer lists, paths.

fn write_ptrs_to_bytes<W: Write>(ptrs: &[TriePtr], w: &mut W) -> Result<(), Error> {
    for ptr in ptrs.iter() {
        ptr.write_bytes(w)?;
    }
    Ok(())
}

fn ptrs_consensus_hash<W: Write, M: BlockMap>(
    ptrs: &[TriePtr],
    map: &mut M,
    w: &mut W,
) -> Result<(), Error> {
    for ptr in ptrs.iter() {
        ptr.write_consensus_bytes(map, w)?;
    }
    Ok(())
}

/// A path in the Trie is the SHA2-512/256 hash of its key.
pub struct TriePath([u8; 32]);
impl_array_newtype!(TriePath, u8, 32);
impl_array_hexstring_fmt!(TriePath);
impl_byte_array_newtype!(TriePath, u8, 32);

pub const TRIEPATH_MAX_LEN: usize = 32;

impl TriePath {
    pub fn from_key(k: &str) -> TriePath {
        let h = TrieHash::from_data(k.as_bytes());
        let mut hb = [0u8; TRIEPATH_MAX_LEN];
        hb.copy_from_slice(h.as_bytes());
        TriePath(hb)
    }
}

/// All Trie nodes implement the following methods:
pub trait TrieNode {
    /// Node ID for encoding/decoding
    fn id(&self) -> u8;

    /// Is the node devoid of children?
    fn empty() -> Self;

    /// Follow a path character to a child pointer
    fn walk(&self, chr: u8) -> Option<TriePtr>;

    /// Insert a child pointer if the path character slot is not occupied.
    /// Return true if inserted, false if the slot is already filled
    fn insert(&mut self, ptr: &TriePtr) -> bool;

    /// Replace an existing child pointer with a new one.  Returns true if replaced; false if the
    /// child does not exist.
    fn replace(&mut self, ptr: &TriePtr) -> bool;

    /// Read an encoded instance of this node from a byte stream and instantiate it.
    fn from_bytes<R: Read>(r: &mut R) -> Result<Self, Error>
    where
        Self: std::marker::Sized;

    /// Get a reference to the children of this node.
    fn ptrs(&self) -> &[TriePtr];

    /// Get a reference to the children of this node.
    fn path(&self) -> &Vec<u8>;

    /// Construct a TrieNodeType from a TrieNode
    fn as_trie_node_type(&self) -> TrieNodeType;

    /// Encode this node instance into a byte stream and write it to w.
    fn write_bytes<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        w.write_all(&[self.id()])?;
        write_ptrs_to_bytes(self.ptrs(), w)?;
        write_path_to_bytes(self.path().as_slice(), w)
    }

    #[cfg(test)]
    fn to_bytes(&self) -> Vec<u8> {
        let mut r = Vec::new();
        self.write_bytes(&mut r)
            .expect("Failed to write to byte buffer");
        r
    }

    /// Calculate how many bytes this node will take to encode.
    fn byte_len(&self) -> usize {
        get_ptrs_byte_len(self.ptrs()) + get_path_byte_len(self.path())
    }
}

/// Trait for types that can serialize to consensus bytes
/// This is implemented by `TrieNode`s and `ProofTrieNode`s
///  and allows hash calculation routines to be the same for
///  both types.
/// The type `M` is used for any additional data structures required
///   (BlockHashMap for TrieNode and () for ProofTrieNode)
pub trait ConsensusSerializable<M> {
    /// Encode the consensus-relevant bytes of this node and write it to w.
    fn write_consensus_bytes<W: Write>(
        &self,
        additional_data: &mut M,
        w: &mut W,
    ) -> Result<(), Error>;

    #[cfg(test)]
    fn to_consensus_bytes(&self, additional_data: &mut M) -> Vec<u8> {
        let mut r = Vec::new();
        self.write_consensus_bytes(additional_data, &mut r)
            .expect("Failed to write to byte buffer");
        r
    }
}

impl<T: TrieNode, M: BlockMap> ConsensusSerializable<M> for T {
    fn write_consensus_bytes<W: Write>(&self, map: &mut M, w: &mut W) -> Result<(), Error> {
        w.write_all(&[self.id()])?;
        ptrs_consensus_hash(self.ptrs(), map, w)?;
        write_path_to_bytes(self.path().as_slice(), w)
    }
}

/// Child pointer
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TriePtr {
    pub id: u8, // ID of the child.  Will have bit 0x80 set if the child is a back-pointer (in which case, back_block will be nonzero)
    pub chr: u8, // Path character at which this child resides
    pub ptr: u32, // Storage-specific pointer to where the child's encoded bytes can be found
    pub back_block: u32, // Pointer back to the block that contains the child, if it's not in this trie
}

pub const TRIEPTR_SIZE: usize = 10; // full size of a TriePtr

pub fn ptrs_fmt(ptrs: &[TriePtr]) -> String {
    let mut strs = vec![];
    for i in 0..ptrs.len() {
        if ptrs[i].id != TrieNodeID::Empty as u8 {
            strs.push(format!(
                "id{}chr{:02x}ptr{}bblk{}",
                ptrs[i].id, ptrs[i].chr, ptrs[i].ptr, ptrs[i].back_block
            ))
        }
    }
    strs.join(",")
}

impl Default for TriePtr {
    #[inline]
    fn default() -> TriePtr {
        TriePtr {
            id: 0,
            chr: 0,
            ptr: 0,
            back_block: 0,
        }
    }
}

impl TriePtr {
    #[inline]
    pub fn new(id: u8, chr: u8, ptr: u32) -> TriePtr {
        TriePtr {
            id: id,
            chr: chr,
            ptr: ptr,
            back_block: 0,
        }
    }

    #[inline]
    pub fn id(&self) -> u8 {
        self.id
    }

    #[inline]
    pub fn chr(&self) -> u8 {
        self.chr
    }

    #[inline]
    pub fn ptr(&self) -> u32 {
        self.ptr
    }

    #[inline]
    pub fn back_block(&self) -> u32 {
        self.back_block
    }

    #[inline]
    pub fn from_backptr(&self) -> TriePtr {
        TriePtr {
            id: clear_backptr(self.id),
            chr: self.chr,
            ptr: self.ptr,
            back_block: 0,
        }
    }

    #[inline]
    pub fn write_bytes<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        w.write_all(&[self.id(), self.chr()])?;
        w.write_all(&self.ptr().to_be_bytes())?;
        w.write_all(&self.back_block().to_be_bytes())?;
        Ok(())
    }

    /// The parts of a child pointer that are relevant for consensus are only its ID, path
    /// character, and referred-to block hash.  The software doesn't care about the details of how/where
    /// nodes are stored.
    pub fn write_consensus_bytes<W: Write, M: BlockMap>(
        &self,
        block_map: &mut M,
        w: &mut W,
    ) -> Result<(), Error> {
        w.write_all(&[self.id(), self.chr()])?;

        if is_backptr(self.id()) {
            w.write_all(
                block_map
                    .get_block_hash_caching(self.back_block())
                    .expect("Block identifier {} refered to an unknown block. Consensus failure.")
                    .as_bytes(),
            )?;
        } else {
            w.write_all(&[0; BLOCK_HEADER_HASH_ENCODED_SIZE])?;
        }
        Ok(())
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> TriePtr {
        assert!(bytes.len() >= TRIEPTR_SIZE);
        let id = bytes[0];
        let chr = bytes[1];
        let ptr = u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]);
        let back_block = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]);

        TriePtr {
            id: id,
            chr: chr,
            ptr: ptr,
            back_block: back_block,
        }
    }
}

/// Cursor structure for walking down one or more Tries.  This structure helps other parts of the
/// codebase remember which nodes were visited, which blocks they came from, and which pointers
/// were walked.  In particular, it's useful for figuring out where to insert a new node, and which
/// nodes to visit when updating the root node hash.
#[derive(Debug, Clone, PartialEq)]
pub struct TrieCursor<T: MarfTrieId> {
    pub path: TriePath,                  // the path to walk
    pub index: usize,                    // index into the path
    pub node_path_index: usize,          // index into the currently-visited node's compressed path
    pub nodes: Vec<TrieNodeType>,        // list of nodes this cursor visits
    pub node_ptrs: Vec<TriePtr>,         // list of ptr branches this cursor has taken
    pub block_hashes: Vec<T>, // list of Tries we've visited.  block_hashes[i] corresponds to node_ptrs[i]
    pub last_error: Option<CursorError>, // last error encountered while walking (used to make sure the client calls the right "recovery" method)
}

impl<T: MarfTrieId> TrieCursor<T> {
    pub fn new(path: &TriePath, root_ptr: TriePtr) -> TrieCursor<T> {
        TrieCursor {
            path: path.clone(),
            index: 0,
            node_path_index: 0,
            nodes: vec![],
            node_ptrs: vec![root_ptr],
            block_hashes: vec![],
            last_error: None,
        }
    }

    /// what point in the path are we at now?
    /// Will be None only if we haven't taken a step yet.
    pub fn chr(&self) -> Option<u8> {
        if self.index > 0 && self.index <= self.path.len() {
            Some(self.path.as_bytes()[self.index - 1])
        } else {
            None
        }
    }

    /// what offset in the path are we at?
    pub fn tell(&self) -> usize {
        self.index
    }

    /// what is the offset in the node's compressed path?
    pub fn ntell(&self) -> usize {
        self.node_path_index
    }

    /// Are we a the [E]nd [O]f [P]ath?
    pub fn eop(&self) -> bool {
        self.index == self.path.len()
    }

    /// last ptr visited
    pub fn ptr(&self) -> TriePtr {
        // should always be true by construction
        assert!(self.node_ptrs.len() > 0);
        self.node_ptrs[self.node_ptrs.len() - 1].clone()
    }

    /// last node visited.
    /// Will only be None if we haven't taken a step yet.
    pub fn node(&self) -> Option<TrieNodeType> {
        match self.nodes.len() {
            0 => None,
            _ => Some(self.nodes[self.nodes.len() - 1].clone()),
        }
    }

    /// Are we at the [E]nd [O]f a [N]ode's [P]ath?
    pub fn eonp(&self, node: &TrieNodeType) -> bool {
        match node {
            TrieNodeType::Leaf(ref data) => self.node_path_index == data.path.len(),
            TrieNodeType::Node4(ref data) => self.node_path_index == data.path.len(),
            TrieNodeType::Node16(ref data) => self.node_path_index == data.path.len(),
            TrieNodeType::Node48(ref data) => self.node_path_index == data.path.len(),
            TrieNodeType::Node256(ref data) => self.node_path_index == data.path.len(),
        }
    }

    /// Walk to the next node, following its compressed path as far as we can and then walking to
    /// its child pointer.  If we successfully follow the path, then return the pointer we reached.
    /// Otherwise, if we reach the end of the path, return None.  If the path diverges or a node
    /// cannot be found, then return an Err.
    ///
    /// This method does not follow back-pointers, and will return Err if a back-pointer is
    /// reached.  The caller will need to manually call walk() on the last node visited to get the
    /// back-pointer, shunt to the node it points to, and then call walk_backptr_step_backptr() to
    /// record the back-pointer that was followed.  Once the back-pointer has been followed,
    /// caller should call walk_backptr_step_finish().  This is specifically relevant to the MARF,
    /// not to the individual tries.
    pub fn walk(
        &mut self,
        node: &TrieNodeType,
        block_hash: &T,
    ) -> Result<Option<TriePtr>, CursorError> {
        // can only be called if we called the appropriate "repair" method or if there is no error
        assert!(self.last_error.is_none());

        trace!("cursor: walk: node = {:?} block = {:?}", node, block_hash);

        // walk this node
        self.nodes.push((*node).clone());
        self.node_path_index = 0;

        if self.index >= self.path.len() {
            trace!("cursor: out of path");
            return Ok(None);
        }

        let node_path = node.path_bytes();
        let path_bytes = self.path.as_bytes();

        // consume as much of the compressed path as we can
        for i in 0..node_path.len() {
            if node_path[i] != path_bytes[self.index] {
                // diverged
                trace!("cursor: diverged({} != {}): i = {}, self.index = {}, self.node_path_index = {}", to_hex(&node_path), to_hex(path_bytes), i, self.index, self.node_path_index);
                self.last_error = Some(CursorError::PathDiverged);
                return Err(CursorError::PathDiverged);
            }
            self.index += 1;
            self.node_path_index += 1;
        }

        // walked to end of the node's compressed path.
        // Find the pointer to the next node.
        if self.index < self.path.len() {
            let chr = path_bytes[self.index];
            self.index += 1;
            let mut ptr_opt = node.walk(chr);

            let do_walk = match ptr_opt {
                Some(ptr) => {
                    if !is_backptr(ptr.id()) {
                        // not going to follow a back-pointer
                        self.node_ptrs.push(ptr);
                        self.block_hashes.push(block_hash.clone());
                        true
                    } else {
                        // the caller will need to follow the backptr, and call
                        // repair_backptr_step_backptr() for each node visited, and then repair_backptr_finish()
                        // once the final ptr and block_hash are discovered.
                        self.last_error = Some(CursorError::BackptrEncountered(ptr));
                        false
                    }
                }
                None => {
                    self.last_error = Some(CursorError::ChrNotFound);
                    false
                }
            };

            if !do_walk {
                ptr_opt = None;
            }

            if ptr_opt.is_none() {
                assert!(self.last_error.is_some());

                trace!(
                    "cursor: not found: chr = 0x{:02x}, self.index = {}, self.path = {:?}",
                    chr,
                    self.index - 1,
                    &path_bytes
                );
                return Err(self.last_error.clone().unwrap());
            } else {
                return Ok(ptr_opt);
            }
        } else {
            trace!("cursor: now out of path");
            return Ok(None);
        }
    }

    /// Replace the last-visited node and ptr within this trie.  Used when doing a copy-on-write or
    /// promoting a node, so the cursor state accurately reflects the nodes and tries visited.
    pub fn repair_retarget(&mut self, node: &TrieNodeType, ptr: &TriePtr, hash: &T) -> () {
        // this can only be called if we failed to walk to a node (this method _should not_ be
        // called if we walked to a backptr).
        if Some(CursorError::ChrNotFound) != self.last_error
            && Some(CursorError::PathDiverged) != self.last_error
        {
            eprintln!("{:?}", &self.last_error);
            panic!();
        }

        self.nodes.pop();
        self.node_ptrs.pop();
        self.block_hashes.pop();

        self.nodes.push(node.clone());
        self.node_ptrs.push(ptr.clone());
        self.block_hashes.push(hash.clone());

        self.last_error = None;
    }

    /// Record that a node was walked to by way of a back-pointer.
    /// next_node should be the node walked to.
    /// ptr is the ptr we'll be walking from, off of next_node.
    /// block_hash is the block where next_node came from.
    pub fn repair_backptr_step_backptr(
        &mut self,
        next_node: &TrieNodeType,
        ptr: &TriePtr,
        block_hash: T,
    ) -> () {
        // this can only be called if we walked to a backptr.
        // If it's anything else, we're in trouble.
        if Some(CursorError::ChrNotFound) == self.last_error
            || Some(CursorError::PathDiverged) == self.last_error
        {
            eprintln!("{:?}", &self.last_error);
            panic!();
        }

        trace!(
            "Cursor: repair_backptr_step_backptr ptr={:?} block_hash={:?} next_node={:?}",
            ptr,
            &block_hash,
            next_node
        );

        let backptr = TriePtr::new(set_backptr(ptr.id()), ptr.chr(), ptr.ptr()); // set_backptr() informs update_root_hash() to skip this node
        self.node_ptrs.push(backptr);
        self.block_hashes.push(block_hash);

        self.nodes.push(next_node.clone());
    }

    /// Record that we landed on a non-backptr from a backptr.
    /// ptr is a non-backptr that refers to the node we landed on.
    pub fn repair_backptr_finish(&mut self, ptr: &TriePtr, block_hash: T) -> () {
        // this can only be called if we walked to a backptr.
        // If it's anything else, we're in trouble.
        if Some(CursorError::ChrNotFound) == self.last_error
            || Some(CursorError::PathDiverged) == self.last_error
        {
            eprintln!("{:?}", &self.last_error);
            panic!();
        }
        assert!(!is_backptr(ptr.id()));

        trace!(
            "Cursor: repair_backptr_finish ptr={:?} block_hash={:?}",
            &ptr,
            &block_hash
        );

        self.node_ptrs.push(ptr.clone());
        self.block_hashes.push(block_hash);

        self.last_error = None;
    }
}

impl PartialEq for TrieLeaf {
    fn eq(&self, other: &TrieLeaf) -> bool {
        self.path == other.path && slice_partialeq(self.data.as_bytes(), other.data.as_bytes())
    }
}

impl TrieLeaf {
    pub fn new(path: &Vec<u8>, data: &Vec<u8>) -> TrieLeaf {
        assert!(data.len() <= 40);
        let mut bytes = [0u8; 40];
        bytes.copy_from_slice(&data[..]);
        TrieLeaf {
            path: path.clone(),
            data: MARFValue(bytes),
        }
    }

    pub fn from_value(path: &Vec<u8>, value: MARFValue) -> TrieLeaf {
        TrieLeaf {
            path: path.clone(),
            data: value,
        }
    }
}

impl StacksMessageCodec for TrieLeaf {
    fn consensus_serialize<W: Write>(&self, fd: &mut W) -> Result<(), ::net::Error> {
        self.path.consensus_serialize(fd)?;
        self.data.consensus_serialize(fd)
    }

    fn consensus_deserialize<R: Read>(fd: &mut R) -> Result<TrieLeaf, ::net::Error> {
        let path = read_next(fd)?;
        let data = read_next(fd)?;

        Ok(TrieLeaf { path, data })
    }
}

/// Trie node with four children
#[derive(Clone, PartialEq)]
pub struct TrieNode4 {
    pub path: Vec<u8>,
    pub ptrs: [TriePtr; 4],
}

impl fmt::Debug for TrieNode4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "TrieNode4(path={} ptrs={})",
            &to_hex(&self.path),
            ptrs_fmt(&self.ptrs)
        )
    }
}

impl TrieNode4 {
    pub fn new(path: &Vec<u8>) -> TrieNode4 {
        TrieNode4 {
            path: path.clone(),
            ptrs: [TriePtr::default(); 4],
        }
    }
}

/// Trie node with 16 children
#[derive(Clone, PartialEq)]
pub struct TrieNode16 {
    pub path: Vec<u8>,
    pub ptrs: [TriePtr; 16],
}

impl fmt::Debug for TrieNode16 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "TrieNode16(path={} ptrs={})",
            &to_hex(&self.path),
            ptrs_fmt(&self.ptrs)
        )
    }
}

impl TrieNode16 {
    pub fn new(path: &Vec<u8>) -> TrieNode16 {
        TrieNode16 {
            path: path.clone(),
            ptrs: [TriePtr::default(); 16],
        }
    }

    /// Promote a Node4 to a Node16
    pub fn from_node4(node4: &TrieNode4) -> TrieNode16 {
        let mut ptrs = [TriePtr::default(); 16];
        for i in 0..4 {
            ptrs[i] = node4.ptrs[i].clone();
        }
        TrieNode16 {
            path: node4.path.clone(),
            ptrs: ptrs,
        }
    }
}

/// Trie node with 48 children
#[derive(Clone)]
pub struct TrieNode48 {
    pub path: Vec<u8>,
    indexes: [i8; 256], // indexes[i], if non-negative, is an index into ptrs.
    pub ptrs: [TriePtr; 48],
}

impl fmt::Debug for TrieNode48 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "TrieNode48(path={} ptrs={})",
            &to_hex(&self.path),
            ptrs_fmt(&self.ptrs)
        )
    }
}

impl PartialEq for TrieNode48 {
    fn eq(&self, other: &TrieNode48) -> bool {
        self.path == other.path
            && slice_partialeq(&self.ptrs, &other.ptrs)
            && slice_partialeq(&self.indexes, &other.indexes)
    }
}

impl TrieNode48 {
    pub fn new(path: &Vec<u8>) -> TrieNode48 {
        TrieNode48 {
            path: path.clone(),
            indexes: [-1; 256],
            ptrs: [TriePtr::default(); 48],
        }
    }

    /// Promote a node16 to a node48
    pub fn from_node16(node16: &TrieNode16) -> TrieNode48 {
        let mut ptrs = [TriePtr::default(); 48];
        let mut indexes = [-1i8; 256];
        for i in 0..16 {
            ptrs[i] = node16.ptrs[i].clone();
            indexes[ptrs[i].chr() as usize] = i as i8;
        }
        TrieNode48 {
            path: node16.path.clone(),
            indexes: indexes,
            ptrs: ptrs,
        }
    }
}

/// Trie node with 256 children
#[derive(Clone)]
pub struct TrieNode256 {
    pub path: Vec<u8>,
    pub ptrs: [TriePtr; 256],
}

impl fmt::Debug for TrieNode256 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "TrieNode256(path={} ptrs={})",
            &to_hex(&self.path),
            ptrs_fmt(&self.ptrs)
        )
    }
}

impl PartialEq for TrieNode256 {
    fn eq(&self, other: &TrieNode256) -> bool {
        self.path == other.path && slice_partialeq(&self.ptrs, &other.ptrs)
    }
}

impl TrieNode256 {
    pub fn new(path: &Vec<u8>) -> TrieNode256 {
        TrieNode256 {
            path: path.clone(),
            ptrs: [TriePtr::default(); 256],
        }
    }

    pub fn from_node4(node4: &TrieNode4) -> TrieNode256 {
        let mut ptrs = [TriePtr::default(); 256];
        for i in 0..4 {
            let c = node4.ptrs[i].chr();
            ptrs[c as usize] = node4.ptrs[i].clone();
        }
        TrieNode256 {
            path: node4.path.clone(),
            ptrs: ptrs,
        }
    }

    /// Promote a node48 to a node256
    pub fn from_node48(node48: &TrieNode48) -> TrieNode256 {
        let mut ptrs = [TriePtr::default(); 256];
        for i in 0..48 {
            let c = node48.ptrs[i].chr();
            ptrs[c as usize] = node48.ptrs[i].clone();
        }
        TrieNode256 {
            path: node48.path.clone(),
            ptrs: ptrs,
        }
    }
}

impl TrieNode for TrieNode4 {
    fn id(&self) -> u8 {
        TrieNodeID::Node4 as u8
    }

    fn empty() -> TrieNode4 {
        TrieNode4 {
            path: vec![],
            ptrs: [TriePtr::default(); 4],
        }
    }

    fn walk(&self, chr: u8) -> Option<TriePtr> {
        for i in 0..4 {
            if self.ptrs[i].id() != TrieNodeID::Empty as u8 && self.ptrs[i].chr() == chr {
                return Some(self.ptrs[i].clone());
            }
        }
        return None;
    }

    fn from_bytes<R: Read>(r: &mut R) -> Result<TrieNode4, Error> {
        let mut ptrs_slice = [TriePtr::default(); 4];
        ptrs_from_bytes(TrieNodeID::Node4 as u8, r, &mut ptrs_slice)?;
        let path = path_from_bytes(r)?;

        Ok(TrieNode4 {
            path,
            ptrs: ptrs_slice,
        })
    }

    fn insert(&mut self, ptr: &TriePtr) -> bool {
        if self.replace(ptr) {
            return true;
        }

        for i in 0..4 {
            if self.ptrs[i].id() == TrieNodeID::Empty as u8 {
                self.ptrs[i] = ptr.clone();
                return true;
            }
        }
        return false;
    }

    fn replace(&mut self, ptr: &TriePtr) -> bool {
        for i in 0..4 {
            if self.ptrs[i].id() != TrieNodeID::Empty as u8 && self.ptrs[i].chr() == ptr.chr() {
                self.ptrs[i] = ptr.clone();
                return true;
            }
        }
        return false;
    }

    fn ptrs(&self) -> &[TriePtr] {
        &self.ptrs
    }

    fn path(&self) -> &Vec<u8> {
        &self.path
    }

    fn as_trie_node_type(&self) -> TrieNodeType {
        TrieNodeType::Node4(self.clone())
    }
}

impl TrieNode for TrieNode16 {
    fn id(&self) -> u8 {
        TrieNodeID::Node16 as u8
    }

    fn empty() -> TrieNode16 {
        TrieNode16 {
            path: vec![],
            ptrs: [TriePtr::default(); 16],
        }
    }

    fn walk(&self, chr: u8) -> Option<TriePtr> {
        for i in 0..16 {
            if self.ptrs[i].id != TrieNodeID::Empty as u8 && self.ptrs[i].chr == chr {
                return Some(self.ptrs[i].clone());
            }
        }
        return None;
    }

    fn from_bytes<R: Read>(r: &mut R) -> Result<TrieNode16, Error> {
        let mut ptrs_slice = [TriePtr::default(); 16];
        ptrs_from_bytes(TrieNodeID::Node16 as u8, r, &mut ptrs_slice)?;

        let path = path_from_bytes(r)?;

        Ok(TrieNode16 {
            path,
            ptrs: ptrs_slice,
        })
    }

    fn insert(&mut self, ptr: &TriePtr) -> bool {
        if self.replace(ptr) {
            return true;
        }

        for i in 0..16 {
            if self.ptrs[i].id() == TrieNodeID::Empty as u8 {
                self.ptrs[i] = ptr.clone();
                return true;
            }
        }
        return false;
    }

    fn replace(&mut self, ptr: &TriePtr) -> bool {
        for i in 0..16 {
            if self.ptrs[i].id() != TrieNodeID::Empty as u8 && self.ptrs[i].chr() == ptr.chr() {
                self.ptrs[i] = ptr.clone();
                return true;
            }
        }
        return false;
    }

    fn ptrs(&self) -> &[TriePtr] {
        &self.ptrs
    }

    fn path(&self) -> &Vec<u8> {
        &self.path
    }

    fn as_trie_node_type(&self) -> TrieNodeType {
        TrieNodeType::Node16(self.clone())
    }
}

impl TrieNode for TrieNode48 {
    fn id(&self) -> u8 {
        TrieNodeID::Node48 as u8
    }

    fn empty() -> TrieNode48 {
        TrieNode48 {
            path: vec![],
            indexes: [-1; 256],
            ptrs: [TriePtr::default(); 48],
        }
    }

    fn walk(&self, chr: u8) -> Option<TriePtr> {
        let idx = self.indexes[chr as usize];
        if idx >= 0 && idx < 48 && self.ptrs[idx as usize].id() != TrieNodeID::Empty as u8 {
            return Some(self.ptrs[idx as usize].clone());
        }
        return None;
    }

    fn write_bytes<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        w.write_all(&[self.id()])?;
        write_ptrs_to_bytes(self.ptrs(), w)?;

        for i in self.indexes.iter() {
            w.write_all(&[*i as u8])?;
        }

        write_path_to_bytes(self.path().as_slice(), w)
    }

    fn byte_len(&self) -> usize {
        get_ptrs_byte_len(&self.ptrs) + 256 + get_path_byte_len(&self.path)
    }

    fn from_bytes<R: Read>(r: &mut R) -> Result<TrieNode48, Error> {
        let mut ptrs_slice = [TriePtr::default(); 48];
        ptrs_from_bytes(TrieNodeID::Node48 as u8, r, &mut ptrs_slice)?;

        let mut indexes = [0u8; 256];
        let l_indexes = r.read(&mut indexes).map_err(Error::IOError)?;

        if l_indexes != 256 {
            return Err(Error::CorruptionError(
                "Node48: Failed to read 256 indexes".to_string(),
            ));
        }

        let path = path_from_bytes(r)?;

        let indexes_i8: Vec<i8> = indexes
            .iter()
            .map(|i| {
                let j = *i as i8;
                j
            })
            .collect();
        let mut indexes_slice = [0i8; 256];
        indexes_slice.copy_from_slice(&indexes_i8[..]);

        // not a for-loop because "for ptr in ptrs_slice.iter()" is actually kinda slow
        let mut i = 0;
        while i < ptrs_slice.len() {
            let ptr = &ptrs_slice[i];
            if !(ptr.id() == TrieNodeID::Empty as u8
                || (indexes_slice[ptr.chr() as usize] >= 0
                    && indexes_slice[ptr.chr() as usize] < 48))
            {
                return Err(Error::CorruptionError(
                    "Node48: corrupt index array: invalid index value".to_string(),
                ));
            }
            i += 1;
        }

        // not a for-loop because "for i in 0..256" is actually kinda slow
        i = 0;
        while i < 256 {
            if !(indexes_slice[i] < 0
                || (indexes_slice[i] >= 0
                    && (indexes_slice[i] as usize) < ptrs_slice.len()
                    && ptrs_slice[indexes_slice[i] as usize].id() != TrieNodeID::Empty as u8))
            {
                return Err(Error::CorruptionError(
                    "Node48: corrupt index array: index points to empty node".to_string(),
                ));
            }
            i += 1;
        }

        Ok(TrieNode48 {
            path,
            indexes: indexes_slice,
            ptrs: ptrs_slice,
        })
    }

    fn insert(&mut self, ptr: &TriePtr) -> bool {
        if self.replace(ptr) {
            return true;
        }

        let c = ptr.chr();
        for i in 0..48 {
            if self.ptrs[i].id() == TrieNodeID::Empty as u8 {
                self.indexes[c as usize] = i as i8;
                self.ptrs[i] = ptr.clone();
                return true;
            }
        }
        return false;
    }

    fn replace(&mut self, ptr: &TriePtr) -> bool {
        let i = self.indexes[ptr.chr() as usize];
        if i >= 0 {
            self.ptrs[i as usize] = ptr.clone();
            return true;
        } else {
            return false;
        }
    }

    fn ptrs(&self) -> &[TriePtr] {
        &self.ptrs
    }

    fn path(&self) -> &Vec<u8> {
        &self.path
    }

    fn as_trie_node_type(&self) -> TrieNodeType {
        TrieNodeType::Node48(Box::new(self.clone()))
    }
}

impl TrieNode for TrieNode256 {
    fn id(&self) -> u8 {
        TrieNodeID::Node256 as u8
    }

    fn empty() -> TrieNode256 {
        TrieNode256 {
            path: vec![],
            ptrs: [TriePtr::default(); 256],
        }
    }

    fn walk(&self, chr: u8) -> Option<TriePtr> {
        if self.ptrs[chr as usize].id() != TrieNodeID::Empty as u8 {
            return Some(self.ptrs[chr as usize].clone());
        }
        return None;
    }

    fn from_bytes<R: Read>(r: &mut R) -> Result<TrieNode256, Error> {
        let mut ptrs_slice = [TriePtr::default(); 256];
        ptrs_from_bytes(TrieNodeID::Node256 as u8, r, &mut ptrs_slice)?;

        let path = path_from_bytes(r)?;

        Ok(TrieNode256 {
            path,
            ptrs: ptrs_slice,
        })
    }

    fn insert(&mut self, ptr: &TriePtr) -> bool {
        if self.replace(ptr) {
            return true;
        }
        let c = ptr.chr() as usize;
        self.ptrs[c] = ptr.clone();
        return true;
    }

    fn replace(&mut self, ptr: &TriePtr) -> bool {
        let c = ptr.chr() as usize;
        if self.ptrs[c].id() != TrieNodeID::Empty as u8 && self.ptrs[c].chr() == ptr.chr() {
            self.ptrs[c] = ptr.clone();
            return true;
        } else {
            return false;
        }
    }

    fn ptrs(&self) -> &[TriePtr] {
        &self.ptrs
    }

    fn path(&self) -> &Vec<u8> {
        &self.path
    }

    fn as_trie_node_type(&self) -> TrieNodeType {
        TrieNodeType::Node256(Box::new(self.clone()))
    }
}

impl TrieLeaf {
    pub fn write_consensus_bytes_leaf<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        self.write_bytes(w)
    }
}

impl TrieNode for TrieLeaf {
    fn id(&self) -> u8 {
        TrieNodeID::Leaf as u8
    }

    fn empty() -> TrieLeaf {
        TrieLeaf::new(&vec![], &[0u8; 40].to_vec())
    }

    fn walk(&self, _chr: u8) -> Option<TriePtr> {
        None
    }

    fn write_bytes<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        w.write_all(&[self.id()])?;
        write_path_to_bytes(&self.path, w)?;
        w.write_all(&self.data.0[..])?;
        Ok(())
    }

    fn byte_len(&self) -> usize {
        1 + get_path_byte_len(&self.path) + self.data.len()
    }

    fn from_bytes<R: Read>(r: &mut R) -> Result<TrieLeaf, Error> {
        let mut idbuf = [0u8; 1];
        let l_idbuf = r.read(&mut idbuf).map_err(Error::IOError)?;

        if l_idbuf != 1 {
            return Err(Error::CorruptionError(
                "Leaf: failed to read ID".to_string(),
            ));
        }

        if clear_backptr(idbuf[0]) != TrieNodeID::Leaf as u8 {
            return Err(Error::CorruptionError(format!(
                "Leaf: bad ID {:x}",
                idbuf[0]
            )));
        }

        let path = path_from_bytes(r)?;
        let mut leaf_data = [0u8; MARF_VALUE_ENCODED_SIZE as usize];
        let l_leaf_data = r.read(&mut leaf_data).map_err(Error::IOError)?;

        if l_leaf_data != (MARF_VALUE_ENCODED_SIZE as usize) {
            return Err(Error::CorruptionError(format!(
                "Leaf: read only {} out of {} bytes",
                l_leaf_data, MARF_VALUE_ENCODED_SIZE
            )));
        }

        Ok(TrieLeaf {
            path: path,
            data: MARFValue(leaf_data),
        })
    }

    fn insert(&mut self, _ptr: &TriePtr) -> bool {
        panic!("can't insert into a leaf");
    }

    fn replace(&mut self, _ptr: &TriePtr) -> bool {
        panic!("can't replace in a leaf");
    }

    fn ptrs(&self) -> &[TriePtr] {
        &[]
    }

    fn path(&self) -> &Vec<u8> {
        &self.path
    }

    fn as_trie_node_type(&self) -> TrieNodeType {
        TrieNodeType::Leaf(self.clone())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrieNodeType {
    Node4(TrieNode4),
    Node16(TrieNode16),
    Node48(Box<TrieNode48>),
    Node256(Box<TrieNode256>),
    Leaf(TrieLeaf),
}

macro_rules! with_node {
    ($self: expr, $pat:pat, $s:expr) => {
        match $self {
            TrieNodeType::Node4($pat) => $s,
            TrieNodeType::Node16($pat) => $s,
            TrieNodeType::Node48($pat) => $s,
            TrieNodeType::Node256($pat) => $s,
            TrieNodeType::Leaf($pat) => $s,
        }
    };
}

impl TrieNodeType {
    pub fn is_leaf(&self) -> bool {
        match self {
            TrieNodeType::Leaf(_) => true,
            _ => false,
        }
    }

    pub fn is_node4(&self) -> bool {
        match self {
            TrieNodeType::Node4(_) => true,
            _ => false,
        }
    }

    pub fn is_node16(&self) -> bool {
        match self {
            TrieNodeType::Node16(_) => true,
            _ => false,
        }
    }

    pub fn is_node48(&self) -> bool {
        match self {
            TrieNodeType::Node48(_) => true,
            _ => false,
        }
    }

    pub fn is_node256(&self) -> bool {
        match self {
            TrieNodeType::Node256(_) => true,
            _ => false,
        }
    }

    pub fn id(&self) -> u8 {
        with_node!(self, ref data, data.id())
    }

    pub fn walk(&self, chr: u8) -> Option<TriePtr> {
        with_node!(self, ref data, data.walk(chr))
    }

    pub fn write_bytes<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        with_node!(self, ref data, data.write_bytes(w))
    }

    pub fn write_consensus_bytes<W: Write, M: BlockMap>(
        &self,
        map: &mut M,
        w: &mut W,
    ) -> Result<(), Error> {
        with_node!(self, ref data, data.write_consensus_bytes(map, w))
    }

    pub fn byte_len(&self) -> usize {
        with_node!(self, ref data, data.byte_len())
    }

    pub fn insert(&mut self, ptr: &TriePtr) -> bool {
        with_node!(self, ref mut data, data.insert(ptr))
    }

    pub fn replace(&mut self, ptr: &TriePtr) -> bool {
        with_node!(self, ref mut data, data.replace(ptr))
    }

    pub fn ptrs(&self) -> &[TriePtr] {
        with_node!(self, ref data, data.ptrs())
    }

    pub fn ptrs_mut(&mut self) -> &mut [TriePtr] {
        match self {
            TrieNodeType::Node4(ref mut data) => &mut data.ptrs,
            TrieNodeType::Node16(ref mut data) => &mut data.ptrs,
            TrieNodeType::Node48(ref mut data) => &mut data.ptrs,
            TrieNodeType::Node256(ref mut data) => &mut data.ptrs,
            TrieNodeType::Leaf(_) => panic!("Leaf has no ptrs"),
        }
    }

    pub fn max_ptrs(&self) -> usize {
        match self {
            TrieNodeType::Node4(_) => 4,
            TrieNodeType::Node16(_) => 16,
            TrieNodeType::Node48(_) => 48,
            TrieNodeType::Node256(_) => 256,
            TrieNodeType::Leaf(_) => 0,
        }
    }

    pub fn path_bytes(&self) -> &Vec<u8> {
        with_node!(self, ref data, &data.path)
    }

    pub fn set_path(&mut self, new_path: Vec<u8>) -> () {
        with_node!(self, ref mut data, data.path = new_path)
    }
}

#[cfg(test)]
mod test {
    #![allow(unused_variables)]
    #![allow(unused_assignments)]

    use std::io::Cursor;

    use chainstate::stacks::index::bits::*;
    use chainstate::stacks::index::marf::*;
    use chainstate::stacks::index::node::*;
    use chainstate::stacks::index::proofs::*;
    use chainstate::stacks::index::storage::*;
    use chainstate::stacks::index::test::*;
    use chainstate::stacks::index::trie::*;

    use super::*;

    #[test]
    fn trieptr_to_bytes() {
        let mut t = TriePtr::new(0x11, 0x22, 0x33445566);
        t.back_block = 0x778899aa;

        let t_bytes = vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa];

        let mut buf = Vec::new();
        t.write_bytes(&mut buf).unwrap();
        assert_eq!(buf, t_bytes);
        assert_eq!(TriePtr::from_bytes(&t_bytes[..]), t);
    }

    #[test]
    fn trie_node4_to_bytes() {
        let mut node4 = TrieNode4::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..3 {
            assert!(node4.insert(&TriePtr::new(
                TrieNodeID::Node16 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }
        let node4_bytes = vec![
            // node ID
            TrieNodeID::Node4 as u8,
            // ptrs (4)
            TrieNodeID::Node16 as u8,
            0x01,
            0x00,
            0x00,
            0x00,
            0x2,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node16 as u8,
            0x02,
            0x00,
            0x00,
            0x00,
            0x3,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node16 as u8,
            0x03,
            0x00,
            0x00,
            0x00,
            0x4,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Empty as u8,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            // path length
            0x14,
            // path
            0x00,
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
            0x09,
            0x0a,
            0x0b,
            0x0c,
            0x0d,
            0x0e,
            0x0f,
            0x10,
            0x11,
            0x12,
            0x13,
        ];
        let mut node4_stream = Cursor::new(node4_bytes.clone());
        let buf = node4.to_bytes();
        assert_eq!(buf, node4_bytes);
        assert_eq!(node4.byte_len(), node4_bytes.len());
        assert_eq!(TrieNode4::from_bytes(&mut node4_stream).unwrap(), node4);
    }

    #[test]
    fn trie_node4_to_consensus_bytes() {
        let mut node4 = TrieNode4::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..3 {
            assert!(node4.insert(&TriePtr::new(
                TrieNodeID::Node16 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }
        let node4_bytes = vec![
            // node ID
            TrieNodeID::Node4 as u8,
            // ptrs (4): ID, chr, block-header-hash
            TrieNodeID::Node16 as u8,
            0x01,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node16 as u8,
            0x02,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node16 as u8,
            0x03,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Empty as u8,
            0x00,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            // path length
            0x14,
            // path
            0x00,
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
            0x09,
            0x0a,
            0x0b,
            0x0c,
            0x0d,
            0x0e,
            0x0f,
            0x10,
            0x11,
            0x12,
            0x13,
        ];

        let buf = node4.to_consensus_bytes(&mut ());
        assert_eq!(to_hex(buf.as_slice()), to_hex(node4_bytes.as_slice()));
    }

    #[test]
    fn trie_node16_to_bytes() {
        let mut node16 = TrieNode16::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..15 {
            assert!(node16.insert(&TriePtr::new(
                TrieNodeID::Node48 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }
        let node16_bytes = vec![
            // node ID
            TrieNodeID::Node16 as u8,
            // ptrs (16)
            TrieNodeID::Node48 as u8,
            0x01,
            0x00,
            0x00,
            0x00,
            0x02,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x02,
            0x00,
            0x00,
            0x00,
            0x03,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x03,
            0x00,
            0x00,
            0x00,
            0x04,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x04,
            0x00,
            0x00,
            0x00,
            0x05,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x05,
            0x00,
            0x00,
            0x00,
            0x06,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x06,
            0x00,
            0x00,
            0x00,
            0x07,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x07,
            0x00,
            0x00,
            0x00,
            0x08,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x08,
            0x00,
            0x00,
            0x00,
            0x09,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x09,
            0x00,
            0x00,
            0x00,
            0x0a,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x0a,
            0x00,
            0x00,
            0x00,
            0x0b,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x0b,
            0x00,
            0x00,
            0x00,
            0x0c,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x0c,
            0x00,
            0x00,
            0x00,
            0x0d,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x0d,
            0x00,
            0x00,
            0x00,
            0x0e,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x0e,
            0x00,
            0x00,
            0x00,
            0x0f,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node48 as u8,
            0x0f,
            0x00,
            0x00,
            0x00,
            0x10,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Empty as u8,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            // path length
            0x14,
            // path
            0x00,
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
            0x09,
            0x0a,
            0x0b,
            0x0c,
            0x0d,
            0x0e,
            0x0f,
            0x10,
            0x11,
            0x12,
            0x13,
        ];
        let mut node16_stream = Cursor::new(node16_bytes.clone());
        let buf = node16.to_bytes();
        assert_eq!(buf, node16_bytes);
        assert_eq!(node16.byte_len(), node16_bytes.len());
        assert_eq!(TrieNode16::from_bytes(&mut node16_stream).unwrap(), node16);
    }

    #[test]
    fn trie_node16_to_consensus_bytes() {
        let mut node16 = TrieNode16::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..15 {
            assert!(node16.insert(&TriePtr::new(
                TrieNodeID::Node48 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }
        let node16_bytes = vec![
            // node ID
            TrieNodeID::Node16 as u8,
            TrieNodeID::Node48 as u8,
            0x01,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x02,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x03,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x04,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x05,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x06,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x07,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x08,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x09,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x0a,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x0b,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x0c,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x0d,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x0e,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node48 as u8,
            0x0f,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Empty as u8,
            0x00,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            // path length
            0x14,
            // path
            0x00,
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
            0x09,
            0x0a,
            0x0b,
            0x0c,
            0x0d,
            0x0e,
            0x0f,
            0x10,
            0x11,
            0x12,
            0x13,
        ];
        let buf = node16.to_consensus_bytes(&mut ());
        assert_eq!(to_hex(buf.as_slice()), to_hex(node16_bytes.as_slice()));
    }

    #[test]
    fn trie_node48_to_bytes() {
        let mut node48 = TrieNode48::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..47 {
            assert!(node48.insert(&TriePtr::new(
                TrieNodeID::Node256 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }

        let node48_bytes = vec![
            // node ID
            TrieNodeID::Node48 as u8,
            // ptrs (48)
            TrieNodeID::Node256 as u8,
            0x01,
            0x00,
            0x00,
            0x00,
            0x02,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x02,
            0x00,
            0x00,
            0x00,
            0x03,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x03,
            0x00,
            0x00,
            0x00,
            0x04,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x04,
            0x00,
            0x00,
            0x00,
            0x05,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x05,
            0x00,
            0x00,
            0x00,
            0x06,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x06,
            0x00,
            0x00,
            0x00,
            0x07,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x07,
            0x00,
            0x00,
            0x00,
            0x08,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x08,
            0x00,
            0x00,
            0x00,
            0x09,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x09,
            0x00,
            0x00,
            0x00,
            0x0a,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x0a,
            0x00,
            0x00,
            0x00,
            0x0b,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x0b,
            0x00,
            0x00,
            0x00,
            0x0c,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x0c,
            0x00,
            0x00,
            0x00,
            0x0d,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x0d,
            0x00,
            0x00,
            0x00,
            0x0e,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x0e,
            0x00,
            0x00,
            0x00,
            0x0f,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x0f,
            0x00,
            0x00,
            0x00,
            0x10,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x10,
            0x00,
            0x00,
            0x00,
            0x11,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x11,
            0x00,
            0x00,
            0x00,
            0x12,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x12,
            0x00,
            0x00,
            0x00,
            0x13,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x13,
            0x00,
            0x00,
            0x00,
            0x14,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x14,
            0x00,
            0x00,
            0x00,
            0x15,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x15,
            0x00,
            0x00,
            0x00,
            0x16,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x16,
            0x00,
            0x00,
            0x00,
            0x17,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x17,
            0x00,
            0x00,
            0x00,
            0x18,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x18,
            0x00,
            0x00,
            0x00,
            0x19,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x19,
            0x00,
            0x00,
            0x00,
            0x1a,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x1a,
            0x00,
            0x00,
            0x00,
            0x1b,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x1b,
            0x00,
            0x00,
            0x00,
            0x1c,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x1c,
            0x00,
            0x00,
            0x00,
            0x1d,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x1d,
            0x00,
            0x00,
            0x00,
            0x1e,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x1e,
            0x00,
            0x00,
            0x00,
            0x1f,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x1f,
            0x00,
            0x00,
            0x00,
            0x20,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x20,
            0x00,
            0x00,
            0x00,
            0x21,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x21,
            0x00,
            0x00,
            0x00,
            0x22,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x22,
            0x00,
            0x00,
            0x00,
            0x23,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x23,
            0x00,
            0x00,
            0x00,
            0x24,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x24,
            0x00,
            0x00,
            0x00,
            0x25,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x25,
            0x00,
            0x00,
            0x00,
            0x26,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x26,
            0x00,
            0x00,
            0x00,
            0x27,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x27,
            0x00,
            0x00,
            0x00,
            0x28,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x28,
            0x00,
            0x00,
            0x00,
            0x29,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x29,
            0x00,
            0x00,
            0x00,
            0x2a,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x2a,
            0x00,
            0x00,
            0x00,
            0x2b,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x2b,
            0x00,
            0x00,
            0x00,
            0x2c,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x2c,
            0x00,
            0x00,
            0x00,
            0x2d,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x2d,
            0x00,
            0x00,
            0x00,
            0x2e,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x2e,
            0x00,
            0x00,
            0x00,
            0x2f,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Node256 as u8,
            0x2f,
            0x00,
            0x00,
            0x00,
            0x30,
            0x00,
            0x00,
            0x00,
            0x00,
            TrieNodeID::Empty as u8,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            0x00,
            // indexes (256)
            255,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10,
            11,
            12,
            13,
            14,
            15,
            16,
            17,
            18,
            19,
            20,
            21,
            22,
            23,
            24,
            25,
            26,
            27,
            28,
            29,
            30,
            31,
            32,
            33,
            34,
            35,
            36,
            37,
            38,
            39,
            40,
            41,
            42,
            43,
            44,
            45,
            46,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            255,
            // path len
            0x14,
            // path
            0x00,
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
            0x09,
            0x0a,
            0x0b,
            0x0c,
            0x0d,
            0x0e,
            0x0f,
            0x10,
            0x11,
            0x12,
            0x13,
        ];
        let mut node48_stream = Cursor::new(node48_bytes.clone());

        let buf = node48.to_bytes();
        assert_eq!(buf, node48_bytes);
        assert_eq!(node48.byte_len(), node48_bytes.len());
        assert_eq!(TrieNode48::from_bytes(&mut node48_stream).unwrap(), node48);
    }

    #[test]
    fn trie_node48_to_consensus_bytes() {
        let mut node48 = TrieNode48::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..47 {
            assert!(node48.insert(&TriePtr::new(
                TrieNodeID::Node256 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }
        let node48_bytes = vec![
            // node ID
            TrieNodeID::Node48 as u8,
            // ptrs (48)
            TrieNodeID::Node256 as u8,
            0x01,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x02,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x03,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x04,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x05,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x06,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x07,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x08,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x09,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x0a,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x0b,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x0c,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x0d,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x0e,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x0f,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x10,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x11,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x12,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x13,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x14,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x15,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x16,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x17,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x18,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x19,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x1a,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x1b,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x1c,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x1d,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x1e,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x1f,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x20,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x21,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x22,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x23,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x24,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x25,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x26,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x27,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x28,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x29,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x2a,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x2b,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x2c,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x2d,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x2e,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Node256 as u8,
            0x2f,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            TrieNodeID::Empty as u8,
            0x00,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            // path len
            0x14,
            // path
            0x00,
            0x01,
            0x02,
            0x03,
            0x04,
            0x05,
            0x06,
            0x07,
            0x08,
            0x09,
            0x0a,
            0x0b,
            0x0c,
            0x0d,
            0x0e,
            0x0f,
            0x10,
            0x11,
            0x12,
            0x13,
        ];
        let buf = node48.to_consensus_bytes(&mut ());
        assert_eq!(buf, node48_bytes);
    }

    #[test]
    fn trie_node256_to_bytes() {
        let mut node256 = TrieNode256::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..255 {
            assert!(node256.insert(&TriePtr::new(
                TrieNodeID::Node256 as u8,
                i as u8,
                (i + 2) % 256
            )));
        }

        let mut node256_bytes = vec![
            // node ID
            TrieNodeID::Node256 as u8,
        ];
        // ptrs (256)
        for i in 0..255 {
            node256_bytes.append(&mut vec![
                TrieNodeID::Node256 as u8,
                i as u8,
                0,
                0,
                0,
                (((i + 2) % 256) as u8),
                0,
                0,
                0,
                0,
            ]);
        }
        // last ptr is empty
        node256_bytes.append(&mut vec![
            TrieNodeID::Empty as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]);
        // path
        node256_bytes.append(&mut vec![
            // path len
            0x14, // path
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13,
        ]);

        let mut node256_stream = Cursor::new(node256_bytes.clone());

        let buf = node256.to_bytes();
        assert_eq!(buf, node256_bytes);
        assert_eq!(node256.byte_len(), node256_bytes.len());
        assert_eq!(
            TrieNode256::from_bytes(&mut node256_stream).unwrap(),
            node256
        );
    }

    #[test]
    fn trie_node256_to_consensus_bytes() {
        let mut node256 = TrieNode256::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..255 {
            assert!(node256.insert(&TriePtr::new(
                TrieNodeID::Node256 as u8,
                i as u8,
                (i + 2) % 256
            )));
        }

        let mut node256_bytes = vec![
            // node ID
            TrieNodeID::Node256 as u8,
        ];
        // ptrs (256)

        let pointer_back_block_bytes = [0; 32];
        for i in 0..255 {
            node256_bytes.append(&mut vec![
                TrieNodeID::Node256 as u8,
                i as u8,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]);
        }
        // last ptr is empty
        node256_bytes.append(&mut vec![
            TrieNodeID::Empty as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]);

        // path
        node256_bytes.append(&mut vec![
            // path len
            0x14, // path
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13,
        ]);

        let buf = node256.to_consensus_bytes(&mut ());
        assert_eq!(buf, node256_bytes);
    }

    #[test]
    fn trie_leaf_to_bytes() {
        let leaf = TrieLeaf::new(
            &vec![
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            ],
            &vec![
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
            ],
        );
        let leaf_bytes = vec![
            // node ID
            TrieNodeID::Leaf as u8,
            // path len
            0x14,
            // path
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10,
            11,
            12,
            13,
            14,
            15,
            16,
            17,
            18,
            19,
            // data
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10,
            11,
            12,
            13,
            14,
            15,
            16,
            17,
            18,
            19,
            20,
            21,
            22,
            23,
            24,
            25,
            26,
            27,
            28,
            29,
            30,
            31,
            32,
            33,
            34,
            35,
            36,
            37,
            38,
            39,
        ];

        let buf = leaf.to_bytes();

        assert_eq!(buf, leaf_bytes);
        assert_eq!(leaf.byte_len(), buf.len());
    }

    #[test]
    fn read_write_node4() {
        let mut node4 = TrieNode4::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..3 {
            assert!(node4.insert(&TriePtr::new(
                TrieNodeID::Node16 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let hash = TrieHash::from_data(&[0u8; 32]);
        let wres = trie_io.write_nodetype(0, &TrieNodeType::Node4(node4.clone()), hash.clone());
        assert!(wres.is_ok());

        let rres = trie_io.read_nodetype(&TriePtr::new(TrieNodeID::Node4 as u8, 0, 0));

        assert!(rres.is_ok());
        assert_eq!(rres.unwrap(), (TrieNodeType::Node4(node4.clone()), hash));
    }

    #[test]
    fn read_write_node16() {
        let mut node16 = TrieNode16::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..16 {
            assert!(node16.insert(&TriePtr::new(
                TrieNodeID::Node48 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }

        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let hash = TrieHash::from_data(&[0u8; 32]);
        let wres = trie_io.write_nodetype(0, &TrieNodeType::Node16(node16.clone()), hash.clone());
        assert!(wres.is_ok());

        let rres = trie_io.read_nodetype(&TriePtr::new(TrieNodeID::Node16 as u8, 0, 0));

        assert!(rres.is_ok());
        assert_eq!(rres.unwrap(), (TrieNodeType::Node16(node16.clone()), hash));
    }

    #[test]
    fn read_write_node48() {
        let mut node48 = TrieNode48::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..48 {
            assert!(node48.insert(&TriePtr::new(
                TrieNodeID::Node256 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }

        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let hash = TrieHash::from_data(&[0u8; 32]);
        let wres = trie_io.write_nodetype(0, &node48.as_trie_node_type(), hash.clone());
        assert!(wres.is_ok());

        let rres = trie_io.read_nodetype(&TriePtr::new(TrieNodeID::Node48 as u8, 0, 0));

        assert!(rres.is_ok());
        assert_eq!(rres.unwrap(), (node48.as_trie_node_type(), hash));
    }

    #[test]
    fn read_write_node256() {
        let mut node256 = TrieNode256::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
        ]);
        for i in 0..256 {
            assert!(node256.insert(&TriePtr::new(
                TrieNodeID::Node256 as u8,
                (i + 1) as u8,
                (i + 2) as u32
            )));
        }

        let hash = TrieHash::from_data(&[0u8; 32]);
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let wres = trie_io.write_nodetype(0, &node256.as_trie_node_type(), hash.clone());
        assert!(wres.is_ok());

        let root_ptr = trie_io.root_ptr();
        let rres =
            trie_io.read_nodetype(&TriePtr::new(TrieNodeID::Node256 as u8, 0, root_ptr as u32));

        assert!(rres.is_ok());
        assert_eq!(rres.unwrap(), (node256.as_trie_node_type(), hash));
    }

    #[test]
    fn read_write_leaf() {
        let leaf = TrieLeaf::new(
            &vec![
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
            ],
            &vec![
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39,
            ],
        );

        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let hash = TrieHash::from_data(&[0u8; 32]);
        let wres = trie_io.write_nodetype(0, &TrieNodeType::Leaf(leaf.clone()), hash.clone());
        assert!(wres.is_ok());

        let rres = trie_io.read_nodetype(&TriePtr::new(TrieNodeID::Leaf as u8, 0, 0));

        assert!(rres.is_ok());
        assert_eq!(rres.unwrap(), (TrieNodeType::Leaf(leaf.clone()), hash));
    }

    #[test]
    fn read_write_node4_hashes() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let mut node4 = TrieNode4::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
        ]);
        let hash = TrieHash::from_data(&[0u8; 32]);

        let mut child_hashes = vec![];
        for i in 0..3 {
            let child = TrieLeaf::new(
                &vec![
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, i as u8,
                ],
                &vec![i as u8; 40],
            );
            let child_hash = get_leaf_hash(&child);

            child_hashes.push(child_hash.clone());

            let ptr = trie_io.last_ptr().unwrap();
            trie_io.write_node(ptr, &child, child_hash).unwrap();
            assert!(node4.insert(&TriePtr::new(TrieNodeID::Leaf as u8, i as u8, ptr)));
        }

        // no final child
        child_hashes.push(TrieHash::from_data(&[]));

        let node4_ptr = trie_io.last_ptr().unwrap();
        let node4_hash = get_node_hash(&node4, &child_hashes, &mut trie_io);
        trie_io.write_node(node4_ptr, &node4, node4_hash).unwrap();

        let read_child_hashes =
            Trie::get_children_hashes(&mut trie_io, &TrieNodeType::Node4(node4)).unwrap();

        assert_eq!(read_child_hashes, child_hashes);
    }

    #[test]
    fn read_write_node16_hashes() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let mut node16 = TrieNode16::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
        ]);
        let hash = TrieHash::from_data(&[0u8; 32]);

        let mut child_hashes = vec![];
        for i in 0..15 {
            let child = TrieLeaf::new(
                &vec![
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, i as u8,
                ],
                &vec![i as u8; 40],
            );
            let child_hash = get_leaf_hash(&child);

            child_hashes.push(child_hash.clone());

            let ptr = trie_io.last_ptr().unwrap();
            trie_io.write_node(ptr, &child, child_hash).unwrap();
            assert!(node16.insert(&TriePtr::new(TrieNodeID::Leaf as u8, i as u8, ptr)));
        }

        // no final child
        child_hashes.push(TrieHash::from_data(&[]));

        let node16_ptr = trie_io.last_ptr().unwrap();
        let node16_hash = get_node_hash(&node16, &child_hashes, &mut trie_io);
        trie_io
            .write_node(node16_ptr, &node16, node16_hash)
            .unwrap();

        let read_child_hashes =
            Trie::get_children_hashes(&mut trie_io, &TrieNodeType::Node16(node16)).unwrap();

        assert_eq!(read_child_hashes, child_hashes);
    }

    #[test]
    fn read_write_node48_hashes() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let mut node48 = TrieNode48::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
        ]);
        let hash = TrieHash::from_data(&[0u8; 32]);

        let mut child_hashes = vec![];
        for i in 0..47 {
            let child = TrieLeaf::new(
                &vec![
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, i as u8,
                ],
                &vec![i as u8; 40],
            );
            let child_hash = get_leaf_hash(&child);

            child_hashes.push(child_hash.clone());

            let ptr = trie_io.last_ptr().unwrap();
            trie_io.write_node(ptr, &child, child_hash).unwrap();
            assert!(node48.insert(&TriePtr::new(TrieNodeID::Leaf as u8, i as u8, ptr)));
        }

        // no final child
        child_hashes.push(TrieHash::from_data(&[]));

        let node48_ptr = trie_io.last_ptr().unwrap();
        let node48_hash = get_node_hash(&node48, &child_hashes, &mut trie_io);
        trie_io
            .write_node(node48_ptr, &node48, node48_hash)
            .unwrap();

        let read_child_hashes =
            Trie::get_children_hashes(&mut trie_io, &TrieNodeType::Node48(Box::new(node48)))
                .unwrap();

        assert_eq!(read_child_hashes, child_hashes);
    }

    #[test]
    fn read_write_node256_hashes() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let mut node256 = TrieNode256::new(&vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
        ]);
        let hash = TrieHash::from_data(&[0u8; 32]);

        let mut child_hashes = vec![];
        for i in 0..255 {
            let child = TrieLeaf::new(
                &vec![
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, i as u8,
                ],
                &vec![i as u8; 40],
            );
            let child_hash = get_leaf_hash(&child);

            child_hashes.push(child_hash.clone());

            let ptr = trie_io.last_ptr().unwrap();
            trie_io.write_node(ptr, &child, child_hash).unwrap();
            assert!(node256.insert(&TriePtr::new(TrieNodeID::Leaf as u8, i as u8, ptr)));
        }

        // no final child
        child_hashes.push(TrieHash::from_data(&[]));

        let node256_ptr = trie_io.last_ptr().unwrap();
        let node256_hash = get_node_hash(&node256, &child_hashes, &mut trie_io);
        trie_io
            .write_node(node256_ptr, &node256, node256_hash)
            .unwrap();

        let read_child_hashes =
            Trie::get_children_hashes(&mut trie_io, &TrieNodeType::Node256(Box::new(node256)))
                .unwrap();

        assert_eq!(read_child_hashes, child_hashes);
    }

    #[test]
    fn trie_cursor_walk_full() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![], 0),
            (vec![], 1),
            (vec![], 2),
            (vec![], 3),
            (vec![], 4),
            (vec![], 5),
            (vec![], 6),
            (vec![], 7),
            (vec![], 8),
            (vec![], 9),
            (vec![], 10),
            (vec![], 11),
            (vec![], 12),
            (vec![], 13),
            (vec![], 14),
            (vec![], 15),
            (vec![], 16),
            (vec![], 17),
            (vec![], 18),
            (vec![], 19),
            (vec![], 20),
            (vec![], 21),
            (vec![], 22),
            (vec![], 23),
            (vec![], 24),
            (vec![], 25),
            (vec![], 26),
            (vec![], 27),
            (vec![], 28),
            (vec![], 29),
            (vec![], 30),
            (vec![], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 32);
        assert_eq!(node_ptrs.len(), 32);
        assert_eq!(hashes.len(), 32);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..31 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[i]);
            assert_eq!(c.tell(), i + 1);
            assert_eq!(c.ntell(), 0);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[31]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(&vec![], &[31u8; 40].to_vec()))
        );
        assert_eq!(hash, hashes[31]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[31]);
        assert_eq!(c.ptr(), node_ptrs[31]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_1() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![0], 1),
            (vec![2], 3),
            (vec![4], 5),
            (vec![6], 7),
            (vec![8], 9),
            (vec![10], 11),
            (vec![12], 13),
            (vec![14], 15),
            (vec![16], 17),
            (vec![18], 19),
            (vec![20], 21),
            (vec![22], 23),
            (vec![24], 25),
            (vec![26], 27),
            (vec![28], 29),
            (vec![30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 16);
        assert_eq!(node_ptrs.len(), 16);
        assert_eq!(hashes.len(), 16);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..15 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[2 * (i + 1) - 1]);
            assert_eq!(c.tell(), 2 * (i + 1));
            assert_eq!(c.ntell(), 1);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[15]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(&vec![30], &[31u8; 40].to_vec()))
        );
        assert_eq!(hash, hashes[15]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[15]);
        assert_eq!(c.ptr(), node_ptrs[15]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_2() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![0, 1], 2),
            (vec![3, 4], 5),
            (vec![6, 7], 8),
            (vec![9, 10], 11),
            (vec![12, 13], 14),
            (vec![15, 16], 17),
            (vec![18, 19], 20),
            (vec![21, 22], 23),
            (vec![24, 25], 26),
            (vec![27, 28], 29),
            (vec![30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 11);
        assert_eq!(node_ptrs.len(), 11);
        assert_eq!(hashes.len(), 11);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..10 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[3 * (i + 1) - 1]);
            assert_eq!(c.tell(), 3 * (i + 1));
            assert_eq!(c.ntell(), 2);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[10]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(&vec![30], &[31u8; 40].to_vec()))
        );
        assert_eq!(hash, hashes[10]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[10]);
        assert_eq!(c.ptr(), node_ptrs[10]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_3() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![0, 1, 2], 3),
            (vec![4, 5, 6], 7),
            (vec![8, 9, 10], 11),
            (vec![12, 13, 14], 15),
            (vec![16, 17, 18], 19),
            (vec![20, 21, 22], 23),
            (vec![24, 25, 26], 27),
            (vec![28, 29, 30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 8);
        assert_eq!(node_ptrs.len(), 8);
        assert_eq!(hashes.len(), 8);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..7 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[4 * (i + 1) - 1]);
            assert_eq!(c.tell(), 4 * (i + 1));
            assert_eq!(c.ntell(), 3);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[7]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(&vec![28, 29, 30], &[31u8; 40].to_vec()))
        );
        assert_eq!(hash, hashes[7]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[7]);
        assert_eq!(c.ptr(), node_ptrs[7]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_4() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![0, 1, 2, 3], 4),
            (vec![5, 6, 7, 8], 9),
            (vec![10, 11, 12, 13], 14),
            (vec![15, 16, 17, 18], 19),
            (vec![20, 21, 22, 23], 24),
            (vec![25, 26, 27, 28], 29),
            (vec![30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 7);
        assert_eq!(node_ptrs.len(), 7);
        assert_eq!(hashes.len(), 7);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..6 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[5 * (i + 1) - 1]);
            assert_eq!(c.tell(), 5 * (i + 1));
            assert_eq!(c.ntell(), 4);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[6]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(&vec![30], &[31u8; 40].to_vec()))
        );
        assert_eq!(hash, hashes[6]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[6]);
        assert_eq!(c.ptr(), node_ptrs[6]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_5() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![0, 1, 2, 3, 4], 5),
            (vec![6, 7, 8, 9, 10], 11),
            (vec![12, 13, 14, 15, 16], 17),
            (vec![18, 19, 20, 21, 22], 23),
            (vec![24, 25, 26, 27, 28], 29),
            (vec![30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 6);
        assert_eq!(node_ptrs.len(), 6);
        assert_eq!(hashes.len(), 6);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..5 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[6 * (i + 1) - 1]);
            assert_eq!(c.tell(), 6 * (i + 1));
            assert_eq!(c.ntell(), 5);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[5]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(&vec![30], &[31u8; 40].to_vec()))
        );
        assert_eq!(hash, hashes[5]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[5]);
        assert_eq!(c.ptr(), node_ptrs[5]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_6() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![0, 1, 2, 3, 4, 5], 6),
            (vec![7, 8, 9, 10, 11, 12], 13),
            (vec![14, 15, 16, 17, 18, 19], 20),
            (vec![21, 22, 23, 24, 25, 26], 27),
            (vec![28, 29, 30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 5);
        assert_eq!(node_ptrs.len(), 5);
        assert_eq!(hashes.len(), 5);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..4 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[7 * (i + 1) - 1]);
            assert_eq!(c.tell(), 7 * (i + 1));
            assert_eq!(c.ntell(), 6);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[4]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(&vec![28, 29, 30], &[31u8; 40].to_vec()))
        );
        assert_eq!(hash, hashes[4]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[4]);
        assert_eq!(c.ptr(), node_ptrs[4]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_10() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9], 10),
            (vec![11, 12, 13, 14, 15, 16, 17, 18, 19, 20], 21),
            (vec![22, 23, 24, 25, 26, 27, 28, 29, 30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 3);
        assert_eq!(node_ptrs.len(), 3);
        assert_eq!(hashes.len(), 3);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..2 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[11 * (i + 1) - 1]);
            assert_eq!(c.tell(), 11 * (i + 1));
            assert_eq!(c.ntell(), 10);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[2]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(
                &vec![22, 23, 24, 25, 26, 27, 28, 29, 30],
                &[31u8; 40].to_vec()
            ))
        );
        assert_eq!(hash, hashes[2]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[2]);
        assert_eq!(c.ptr(), node_ptrs[2]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_20() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();
        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![
            (
                vec![
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19,
                ],
                20,
            ),
            (vec![21, 22, 23, 24, 25, 26, 27, 28, 29, 30], 31),
        ];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 2);
        assert_eq!(node_ptrs.len(), 2);
        assert_eq!(hashes.len(), 2);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let mut walk_point = nodes[0].clone();

        for i in 0..1 {
            let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
            assert!(res.is_ok());

            let fields_opt = res.unwrap();
            assert!(fields_opt.is_some());

            let (ptr, node, hash) = fields_opt.unwrap();
            assert_eq!(ptr, node_ptrs[i]);
            assert_eq!(hash, hashes[i]);
            assert_eq!(node, nodes[i + 1]);

            assert_eq!(c.node().unwrap(), nodes[i]);
            assert_eq!(c.ptr(), node_ptrs[i]);
            assert_eq!(c.chr().unwrap(), path[21 * (i + 1) - 1]);
            assert_eq!(c.tell(), 21 * (i + 1));
            assert_eq!(c.ntell(), 20);
            assert!(c.eonp(&c.node().unwrap()));

            walk_point = node;
        }

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[1]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(
                &vec![21, 22, 23, 24, 25, 26, 27, 28, 29, 30],
                &[31u8; 40].to_vec()
            ))
        );
        assert_eq!(hash, hashes[1]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[1]);
        assert_eq!(c.ptr(), node_ptrs[1]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }

    #[test]
    fn trie_cursor_walk_32() {
        let mut trie_io_store = TrieFileStorage::new_memory().unwrap();
        let mut trie_io = trie_io_store.transaction().unwrap();

        trie_io
            .extend_to_block(&BlockHeaderHash([0u8; 32]))
            .unwrap();

        let path_segments = vec![(
            vec![
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30,
            ],
            31,
        )];
        let path = vec![
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ];

        let (nodes, node_ptrs, hashes) =
            make_node4_path(&mut trie_io, &path_segments, [31u8; 40].to_vec());

        assert_eq!(nodes.len(), 1);
        assert_eq!(node_ptrs.len(), 1);
        assert_eq!(hashes.len(), 1);

        assert_eq!(node_ptrs[node_ptrs.len() - 1].chr, 31);
        assert_eq!(node_ptrs[node_ptrs.len() - 1].id, TrieNodeID::Leaf as u8);

        // walk down the trie
        let mut c = TrieCursor::new(
            &TriePath::from_bytes(&path).unwrap(),
            trie_io.root_trieptr(),
        );
        let walk_point = nodes[0].clone();

        // walk to the leaf
        let res = Trie::walk_from(&mut trie_io, &walk_point, &mut c);
        assert!(res.is_ok());

        let fields_opt = res.unwrap();
        assert!(fields_opt.is_some());

        let (ptr, node, hash) = fields_opt.unwrap();
        assert_eq!(ptr, node_ptrs[0]);
        assert_eq!(
            node,
            TrieNodeType::Leaf(TrieLeaf::new(
                &vec![
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                    22, 23, 24, 25, 26, 27, 28, 29, 30
                ],
                &[31u8; 40].to_vec()
            ))
        );
        assert_eq!(hash, hashes[0]);

        // cursor's last-visited node points at the penultimate node (the last node4),
        // but its ptr() is the pointer to the leaf.
        assert_eq!(c.node().unwrap(), nodes[0]);
        assert_eq!(c.ptr(), node_ptrs[0]);
        assert_eq!(c.chr(), Some(path[path.len() - 1]));
        assert_eq!(c.tell(), 32);
        assert!(c.eop());
        assert!(c.eonp(&c.node().unwrap()));

        dump_trie(&mut trie_io);
    }
}
