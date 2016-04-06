#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""
    Blockstack-client
    ~~~~~
    copyright: (c) 2014-2015 by Halfmoon Labs, Inc.
    copyright: (c) 2016 by Blockstack.org

    This file is part of Blockstack-client.

    Blockstack-client is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    Blockstack-client is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.
    You should have received a copy of the GNU General Public License
    along with Blockstack-client. If not, see <http://www.gnu.org/licenses/>.
"""

import argparse
import sys
import json
import traceback
import types
import socket
import uuid
import os
import importlib
import pprint
import random
import time
import copy
import blockstack_profiles
import zone_file
import urllib

from proxy import *
from keys import *
import storage
import user as user_db

import pybitcoin
import bitcoin
import binascii
from utilitybelt import is_hex

from config import get_logger, DEBUG, MAX_RPC_LEN, find_missing, BLOCKSTACKD_SERVER, \
    BLOCKSTACKD_PORT, BLOCKSTACK_METADATA_DIR, BLOCKSTACK_DEFAULT_STORAGE_DRIVERS, \
    FIRST_BLOCK_MAINNET, NAME_OPCODES, OPFIELDS, CONFIG_DIR, SPV_HEADERS_PATH, BLOCKCHAIN_ID_MAGIC, \
    NAME_PREORDER, NAME_REGISTRATION, NAME_UPDATE, NAME_TRANSFER, NAMESPACE_PREORDER, NAME_IMPORT, \
    USER_ZONEFILE_TTL, CONFIG_PATH

log = get_logger()

import virtualchain


def load_name_zonefile(expected_zonefile_hash):
    """
    Fetch and load a user zonefile from the storage implementation with the given hex string hash,
    The user zonefile hash should have been loaded from the blockchain, and thereby be the
    authentic hash.

    Return the user zonefile on success
    Return None on error
    """

    zonefile_txt = storage.get_immutable_data(expected_zonefile_hash, hash_func=storage.get_user_zonefile_hash, deserialize=False)
    if zonefile_txt is None:
        log.error("Failed to load user zonefile '%s'" % expected_zonefile_hash)
        return None

    try:
        # by default, it's a zonefile-formatted text file
        user_zonefile = zone_file.parse_zone_file( zonefile_txt )
        assert user_db.is_user_zonefile( user_zonefile ), "Not a user zonefile: %s" % user_zonefile
    except (IndexError, ValueError, zone_file.InvalidLineException):
        # might be legacy profile
        log.debug("WARN: failed to parse user zonefile; trying to import as legacy")
        try:
            user_zonefile = json.loads(zonefile_txt)
        except Exception, e:
            log.exception(e)
            log.error("Failed to parse:\n%s" % zonefile_txt)
            return None
        
    except Exception, e:
        log.exception(e)
        log.error("Failed to parse:\n%s" % zonefile_txt)
        return None 

    return user_zonefile


def load_legacy_user_profile( name, expected_hash ):
    """
    Load a legacy user profile, and convert it into
    the new zonefile-esque profile format that can 
    be serialized into a JWT.

    Verify that the profile hashses to the above expected hash
    """

    # fetch... 
    storage_host = "onename.com"
    assert name.endswith(".id")

    name_without_namespace = ".".join( name.split(".")[:-1] )
    storage_path = "/%s.json" % name_without_namespace 

    try:
        req = httplib.HTTPConnection( storage_host )
        resp = req.request( "GET", storage_path )
        data = resp.read()
    except Exception, e:
        log.error("Failed to fetch http://%s/%s: %s" % (storage_host, storage_path, e))
        return None 

    try:
        data_json = json.loads(data)
    except Exception, e:
        log.error("Unparseable profile data")
        return None

    data_hash = registrar.utils.get_hash( data_json )
    if expected_hash != data_hash:
        log.error("Hash mismatch: expected %s, got %s" % (expected_hash, data_hash))
        return None

    assert blockstack_profiles.is_profile_in_legacy_format( data_json )
    new_profile = blockstack_profiles.get_person_from_legacy_format( data_json )
    return new_profile
    

def load_name_profile(name, user_zonefile, public_key):
    """
    Fetch and load a user profile, given the user zonefile.

    Return the user profile on success
    Return None on error
    """
    
    urls = user_db.user_zonefile_urls( user_zonefile )
    user_profile = storage.get_mutable_data( name, public_key, urls=urls )
    return user_profile


def profile_update( name, new_profile, proxy=None, wallet_keys=None ):
    """
    Set the new profile data.  CLIENTS SHOULD NOT CALL THIS METHOD DIRECTLY.
    Return {'status: True} on success, as well as {'transaction_hash': hash} if we updated on the blockchain.
    Return {'error': ...} on failure.
    """
    
    ret = {}
    if proxy is None:
        proxy = get_default_proxy()

    # update the profile with the new zonefile
    _, data_privkey = get_data_keypair( wallet_keys=wallet_keys )
    rc = storage.put_mutable_data( name, new_profile, data_privkey )
    if not rc:
        ret['error'] = 'Failed to update profile'
        return ret

    else:
        ret['status'] = True

    return ret


def get_name_zonefile( name, create_if_absent=False, proxy=None, value_hash=None, wallet_keys=None ):
    """
    Given the name of the user, go fetch its zonefile.

    Returns a dict with the zonefile, or 
    a dict with "error" defined and a message.
    Return None if there is no zonefile (i.e. the hash is null)
    """
    if proxy is None:
        proxy = get_default_proxy()

    if value_hash is None:
        # find name record first
        name_record = get_name_blockchain_record(name, proxy=proxy)

        if name_record is None:
            # failed to look up
            return {'error': "No such name"}

        if len(name_record) == 0:
            return {"error": "No such name"}

        # sanity check
        if 'value_hash' not in name_record:
            return {"error": "Name has no user record hash defined"}

        value_hash = name_record['value_hash']

    # is there a user record loaded?
    if value_hash in [None, "null", ""]:

        # no user data
        if not create_if_absent:
            return None

        else:
            # make an empty zonefile and return that
            # get user's data public key 
            public_key, _ = get_data_keypair(wallet_keys=wallet_keys)
            user_resp = user_db.make_empty_user_zonefile(name, public_key)
            return user_resp

    user_zonefile_hash = value_hash
    user_zonefile = load_name_zonefile(user_zonefile_hash)
    if user_zonefile is None:
        return {"error": "Failed to load zonefile"}

    return user_zonefile
    

def get_name_profile(name, create_if_absent=False, proxy=None, wallet_keys=None, user_zonefile=None):
    """
    Given the name of the user, look up the user's record hash,
    and then get the record itself from storage.

    If the user's zonefile is really a legacy profile, then 
    the profile will be the converted legacy profile.  The
    returned zonefile will still be a legacy profile, however.
    The caller can check this and perform the conversion automatically.

    Returns (profile, zonefile) on success.
    Returns (None, {'error': ...}) on failure
    """

    if proxy is None:
        proxy = get_default_proxy()
 
    if user_zonefile is None:
        user_zonefile = get_name_zonefile( name, create_if_absent=create_if_absent, proxy=proxy, wallet_keys=wallet_keys )
        if user_zonefile is None:
            return (None, {'error': 'No user zonefile'})

        if 'error' in user_zonefile:
            return (None, user_zonefile)

    # is this really a legacy profile?
    if blockstack_profiles.is_profile_in_legacy_format( user_zonefile ):
        # convert it 
        user_profile = blockstack_profiles.get_person_from_legacy_format( user_zonefile )
       
    else:
        # get user's data public key 
        user_data_pubkey = user_db.user_zonefile_data_pubkey( user_zonefile )
        if user_data_pubkey is None:
            return (None, {'error': 'No data public key found in user profile.'})

        user_profile = load_name_profile( name, user_zonefile, user_data_pubkey )
        if user_profile is None or 'error' in user_profile:
            if create_if_absent:
                user_profile = user_db.make_empty_user_profile()
            else:
                return (None, {'error': 'Failed to load user profile'})

    return (user_profile, user_zonefile)


def store_name_zonefile( name, user_zonefile, txid ):
    """
    Store JSON user zonefile data to the immutable storage providers, synchronously.
    This is only necessary if we've added/changed/removed immutable data.

    Return (True, hash(user)) on success
    Return (False, None) on failure
    """

    assert not blockstack_profiles.is_profile_in_legacy_format(user_zonefile), "User zonefile is a legacy profile"

    # make sure our data pubkey is there 
    user_data_pubkey = user_db.user_zonefile_data_pubkey( user_zonefile )
    assert user_data_pubkey is not None, "BUG: user zonefile is missing data public key"

    # serialize and send off
    user_zonefile_txt = zone_file.make_zone_file( user_zonefile, origin=name, ttl=USER_ZONEFILE_TTL )
    data_hash = storage.get_user_zonefile_hash( user_zonefile_txt )
    result = storage.put_immutable_data(None, txid, data_hash=data_hash, data_text=user_zonefile_txt )

    rc = None
    if result is None:
        rc = False
    else:
        rc = True

    return (rc, data_hash)


def store_name_profile(username, user_profile, wallet_keys=None):
    """
    Store JSON user profile data to the mutable storage providers, synchronously.

    The wallet must be initialized before calling this.

    Return True on success
    Return False on error
    """

    _, data_privkey = get_data_keypair(wallet_keys=wallet_keys)
    rc = storage.put_mutable_data( username, user_profile, data_privkey )
    return rc


def remove_name_zonefile(user, txid):
    """
    Delete JSON user zonefile data from immutable storage providers, synchronously.

    Return (True, hash(user)) on success
    Return (False, hash(user)) on error
    """

    # serialize
    user_json = json.dumps(user, sort_keys=True)
    data_hash = storage.get_data_hash(user_json)
    result = storage.delete_immutable_data(data_hash, txid)

    rc = None
    if result is None:
        rc = False
    else:
        rc = True

    return (rc, data_hash)


def get_and_migrate_profile( name, proxy=None, create_if_absent=False, wallet_keys=None ):
    """
    Get a name's profile and zonefile, optionally creating a new one along the way.  Migrate the profile to a new zonefile,
    if the profile is in legacy format.

    Return (user_profile, user_zonefile, migrated:bool) on success
    Return ({'error': ...}, None, False) on error
    """

    created_new_zonefile = False
    user_zonefile = get_name_zonefile( name, proxy=proxy, wallet_keys=wallet_keys )
    if user_zonefile is None: 
        if not create_if_absent:
            return ({'error': 'No such zonefile'}, None, False)

        log.debug("Creating new profile and zonefile for name '%s'" % name)
        data_pubkey, _ = get_data_keypair( wallet_keys=wallet_keys )
        user_profile = user_db.make_empty_user_profile()
        user_zonefile = user_db.make_empty_user_zonefile( name, data_pubkey )

        created_new_zonefile = True
    
    elif blockstack_profiles.is_profile_in_legacy_format( user_zonefile ):
        log.debug("Migrating legacy profile to modern zonefile for name '%s'" % name)
        data_pubkey, _ = get_data_keypair( wallet_keys=wallet_keys )
        user_profile = blockstack_profiles.get_person_from_legacy_format( user_zonefile )
        user_zonefile = user_db.make_empty_user_zonefile( name, data_pubkey )

        created_new_zonefile = True

    else:
        user_profile, error_msg = get_name_profile( name, proxy=proxy, wallet_keys=wallet_keys, user_zonefile=user_zonefile )
        if user_profile is None:
            return (error_msg, None, False)

    return (user_profile, user_zonefile, created_new_zonefile)


def migrate_profile( name, txid=None, proxy=None, wallet_keys=None ):
    """
    Migrate a user's profile from the legacy format to the profile/zonefile format.
    Return {'status': True, 'transaction_hash': txid, 'zonefile_hash': ...} on success, if the profile was migrated
    Return {'status': True} on success, if the profile is already migrated
    Return {'error': ...} on error
    """
    legacy = False
    txid = None 
    value_hash = None
    if proxy is None:
        proxy = get_default_proxy()

    user_profile, user_zonefile, legacy = get_and_migrate_profile( name, create_if_absent=True, proxy=proxy, wallet_keys=wallet_keys )
    if 'error' in user_profile:
        log.debug("Unable to load user zonefile for '%s'" % name)
        return user_profile

    if not legacy:
        return {'status': True}

    # store profile...
    _, data_privkey = get_data_keypair( wallet_keys=wallet_keys )
    rc = storage.put_mutable_data( name, user_profile, data_privkey )
    if not rc:
        return {'error': 'Failed to move legacy profile to profile zonefile'}

    # store zonefile, if we haven't already
    if txid is None:
        _, owner_privkey = get_owner_keypair(wallet_keys=wallet_keys)
        update_result = update( name, user_zonefile, owner_privkey, proxy=proxy )
        if 'error' in update_result:
            # failed to replicate user zonefile hash 
            # the caller should simply try again, with the 'transaction_hash' given in the result.
            return update_result

        txid = update_result['transaction_hash']
        value_hash = update_result['value_hash']

    result = {
        'status': True
    }
    if txid is not None:
        result['transaction_hash'] = txid
    if value_hash is not None:
        result['zonefile_hash'] = value_hash

    return result

