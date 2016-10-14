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
    along with Blockstack-client.  If not, see <http://www.gnu.org/licenses/>.
"""

import socket
import base64

import storage
import config

from .config import BLOCKSTACK_TEST

log = config.get_logger()


def url_to_uri_record(url, datum_name=None):
    """
    Convert a URL into a DNS URI record
    """
    try:
        scheme, _ = url.split('://')
    except ValueError:
        msg = 'BUG: invalid storage driver implementation: no scheme given in "{}"'
        raise Exception(msg.format(url))

    scheme = scheme.lower()
    proto = None

    # tcp or udp?
    try:
        port = socket.getservbyname(scheme, 'tcp')
        proto = 'tcp'
    except socket.error:
        try:
            port = socket.getservbyname(scheme, 'udp')
            proto = 'udp'
        except socket.error:
            # this is weird--maybe it's embedded in the scheme?
            try:
                assert len(scheme.split('+')) == 2
                scheme, proto = scheme.split('+')
            except (AssertionError, ValueError):
                msg = 'WARN: Scheme "{}" has no known transport protocol'
                log.debug(msg.format(scheme))

    name = None
    if proto is not None:
        name = '_{}._{}'.format(scheme, proto)
    else:
        name = '_{}'.format(scheme)

    if datum_name is not None:
        name = '{}.{}'.format(name, str(datum_name))

    ret = {
        'name': name,
        'priority': 10,
        'weight': 1,
        'target': url,
    }

    return ret


def make_empty_user_zonefile(username, data_pubkey, urls=None):
    """
    Create an empty user record from a name record.
    """

    # make a URI record for every mutable storage provider
    urls = storage.make_mutable_data_urls(username) if urls is None else urls

    assert urls, 'No profile URLs'

    user = {
        'txt': [],
        'uri': [],
        '$origin': username,
        '$ttl': config.USER_ZONEFILE_TTL,
    }

    if data_pubkey is not None:
        user.setdefault('txt', [])

        pubkey = str(data_pubkey)
        name_txt = {'name': 'pubkey', 'txt': 'pubkey:data:{}'.format(pubkey)}

        user['txt'].append(name_txt)

    for url in urls:
        urirec = url_to_uri_record(url)
        user['uri'].append(urirec)

    return user


def is_user_zonefile(d):
    """
    Is the given dict (or dict-like object) a user zonefile?
    * the zonefile must have a URI record
    """

    if not isinstance(d, dict):
        log.error('Not a dict-like object')
        return False

    if 'uri' not in d:
        return False

    txts = d.get('txt', None)
    if not isinstance(txts, list):
        log.error('txt is not a list-like object')
        return False

    for txt in txts:
        if not isinstance(txt, dict):
            log.error('txt item is not a dict-like object')
            return False

        if 'name' not in txt:
            return False

        if 'txt' not in txt:
            return False

    uris = d.get('uri', None)
    if not isinstance(uris, list):
        log.error('uri is not a list-like object')
        return False

    for uri in uris:
        if not isinstance(uri, dict):
            log.error('uri item is not a dict-like object')
            return False

        if 'name' not in uri:
            return False

        if 'priority' not in uri:
            return False

        if 'weight' not in uri:
            return False

        if 'target' not in uri:
            return False

    return True


def user_zonefile_data_pubkey(user_zonefile, key_prefix='pubkey:data:'):
    """
    Get a user's data public key from their zonefile.
    Return None if not defined
    Raise if there are multiple ones.
    """
    if 'txt' not in user_zonefile:
        return None

    data_pubkey = None
    # check that there is only one of these
    for txtrec in user_zonefile['txt']:
        if not txtrec['txt'].startswith(key_prefix):
            continue

        if data_pubkey is not None:
            msg = 'Multiple data pubkeys'
            log.error('BUG: {}'.format(msg))
            raise ValueError('{} starting with "{}"'.format(msg, key_prefix))

        data_pubkey = txtrec['txt'][len(key_prefix):]

    return data_pubkey


def user_zonefile_set_data_pubkey(user_zonefile, pubkey_hex, key_prefix='pubkey:data:'):
    """
    Set the data public key in the zonefile.
    NOTE: you will need to re-sign all your data!
    """
    user_zonefile.setdefault('txt', [])

    txt = '{}{}'.format(key_prefix, str(pubkey_hex))

    for txtrec in user_zonefile['txt']:
        if txtrec['txt'].startswith(key_prefix):
            # overwrite
            txtrec['txt'] = txt
            return True

    # not present.  add.
    name_txt = {'name': 'pubkey', 'txt': txt}
    user_zonefile['txt'].append(name_txt)

    return True


def user_zonefile_urls(user_zonefile):
    """
    Given a user's zonefile, get the profile URLs
    """
    if 'uri' not in user_zonefile:
        return None

    ret = []
    for urirec in user_zonefile['uri']:
        if 'target' in urirec:
            ret.append(urirec['target'].strip('"'))

    return ret


def add_user_zonefile_url(user_zonefile, url):
    """
    Add a url to a zonefile
    Return the new zonefile on success
    Return None on error or on duplicate URL
    """
    user_zonefile.setdefault('uri', [])

    # avoid duplicates
    for urirec in user_zonefile['uri']:
        target = urirec.get('target', '')
        if target.strip('"') == url:
            return None

    new_urirec = url_to_uri_record(url)
    user_zonefile['uri'].append(new_urirec)

    return user_zonefile


def remove_user_zonefile_url(user_zonefile, url):
    """
    Remove a url from a zonefile
    Return the new zonefile on success
    Return None on error.
    """
    if 'uri' not in user_zonefile:
        return None

    for urirec in user_zonefile['uri']:
        target = urirec.get('target', '')
        if target.strip('"') == url:
            user_zonefile['uri'].remove(urirec)

    return user_zonefile


def make_empty_user_profile():
    """
    Given a user's name, create an empty profile.
    """
    ret = {
        '@type': 'Person',
        'accounts': [],
    }

    return ret


def put_immutable_data_zonefile(user_zonefile, data_id, data_hash, data_url=None):
    """
    Add a data hash to a user's zonefile.  Make sure it's a valid hash as well.
    Return True on success
    Return False otherwise.
    """

    data_hash = str(data_hash)
    assert storage.is_valid_hash(data_hash)

    k = get_immutable_data_hash(user_zonefile, data_id)
    if k is not None:
        # exists or name collision
        return k == data_hash

    txtrec = '#{}'.format(data_hash)
    if data_url is not None:
        txtrec = '{}{}'.format(data_url, txtrec)

    user_zonefile.setdefault('txt', [])

    name_txt = {'name': data_id, 'txt': txtrec}
    user_zonefile['txt'].append(name_txt)

    return True


def get_immutable_hash_from_txt(txtrec):
    """
    Given an immutable data txt record,
    get the hash.
    The hash is the suffix that begins with #.
    Return None if invalid or not present
    """
    if '#' not in txtrec:
        return None

    h = txtrec.split('#')[-1]
    if not storage.is_valid_hash(h):
        return None

    return h


def get_immutable_url_from_txt(txtrec):
    """
    Given an immutable data txt record,
    get the URL hint.
    This is everything that starts before the last #.
    Return None if there is no URL, or we can't parse the txt record
    """
    if '#' not in txtrec:
        return None

    url = '#'.join(txtrec.split('#')[:-1])

    return url or None


def remove_immutable_data_zonefile(user_zonefile, data_hash):
    """
    Remove a data hash from a user's zonefile.
    Return True if removed
    Return False if not present
    """

    data_hash = str(data_hash)
    assert storage.is_valid_hash(data_hash), 'Invalid data hash "{}"'.format(data_hash)

    if 'txt' not in user_zonefile:
        return False

    for txtrec in user_zonefile['txt']:
        h = None
        try:
            h = get_immutable_hash_from_txt(txtrec['txt'])
            assert storage.is_valid_hash(h)
        except:
            # TODO: Check if we should use more fine-grained exception handling
            continue

        if data_hash == h:
            user_zonefile['txt'].remove(txtrec)
            return True

    return False


def has_immutable_data(user_zonefile, data_hash):
    """
    Does the given user have the given immutable data?
    Return True if so
    Return False if not
    """

    data_hash = str(data_hash)
    assert storage.is_valid_hash(data_hash), 'Invalid data hash "{}"'.format(data_hash)

    if 'txt' not in user_zonefile:
        return False

    for txtrec in user_zonefile['txt']:
        h = None
        try:
            h = get_immutable_hash_from_txt(txtrec['txt'])
            assert storage.is_valid_hash(h)
        except:
            # TODO: Check if we should use more fine-grained exception handling
            continue

        if data_hash == h:
            return True

    return False


def has_immutable_data_id(user_zonefile, data_id):
    """
    Does the given user have the given immutable data?
    Return True if so
    Return False if not
    """
    if 'txt' not in user_zonefile:
        return False

    for txtrec in user_zonefile['txt']:
        d_id = None
        try:
            d_id = txtrec['name']
            h = get_immutable_hash_from_txt(txtrec['txt'])
            assert storage.is_valid_hash(h)
        except AssertionError:
            continue

        if data_id == d_id:
            return True

    return False


def get_immutable_data_hash(user_zonefile, data_id):
    """
    Get the hash of an immutable datum by name.
    Return None if there is no match.
    Return the hash if there is one match.
    Return the list of hashes if there are multiple matches.
    """

    if 'txt' not in user_zonefile:
        return None

    ret = None
    for txtrec in user_zonefile['txt']:
        h, d_id = None, None

        try:
            d_id = txtrec['name']
            if data_id != d_id:
                continue

            h = get_immutable_hash_from_txt(txtrec['txt'])
            if h is None:
                log.error('Missing or invalid data hash for "{}"'.format(d_id))

            msg = 'Invalid data hash for "{}" (got "{}" from {})'
            assert storage.is_valid_hash(h), msg.format(d_id, h, txtrec['txt'])
        except AssertionError as ae:
            if BLOCKSTACK_TEST is not None:
                log.exception(ae)

            continue

        if ret is None:
            ret = h
        elif not isinstance(ret, list):
            ret = [ret, h]
        else:
            ret.append(h)

    # NOTE: ret can be one of these types: None, str or list
    return ret


def get_immutable_data_url(user_zonefile, data_hash):
    """
    Given the hash of an immutable datum, find the associated
    URL hint (if given)
    Return None if not given, or not found.
    """

    if 'txt' not in user_zonefile:
        return None

    for txtrec in user_zonefile['txt']:
        h = None
        try:
            h = get_immutable_hash_from_txt(txtrec['txt'])
            if data_hash != h:
                continue

            url = get_immutable_url_from_txt(txtrec['txt'])
        except:
            # TODO: Check if we should use more fine-grained exception handling
            continue

        return url

    return None


def list_immutable_data(user_zonefile):
    """
    Get the IDs and hashes of all immutable data
    Return [(data ID, hash)]
    """
    ret = []
    if 'txt' not in user_zonefile:
        return ret

    for txtrec in user_zonefile['txt']:
        try:
            d_id = txtrec['name']
            h = get_immutable_hash_from_txt(txtrec['txt'])
            assert storage.is_valid_hash(h)
            ret.append((d_id, h))
        except:
            # TODO: Check if we should use more fine-grained exception handling
            continue

    return ret


def has_mutable_data(user_profile, data_id):
    """
    Does the given user profile have the named mutable data defined?
    Return True if so
    Return False if not
    """
    if 'data' not in user_profile:
        return False

    for packed_data_txt in user_profile['data']:
        unpacked_data_id, version = unpack_mutable_data_md(packed_data_txt)
        if data_id == unpacked_data_id:
            return True

    return False


def get_mutable_data_profile(user_profile, data_id):
    """
    Get info for a piece of mutable data, given
    the user's profile and data_id.
    Return the route (as a dict) on success
    Return None if not found
    """

    if 'data' not in user_profile:
        return None

    packed_prefix = pack_mutable_data_md_prefix(data_id)
    for packed_data_txt in user_profile['data']:
        if not packed_data_txt.startswith(packed_prefix):
            continue

        unpacked_data_id, version = unpack_mutable_data_md(packed_data_txt)
        if data_id == unpacked_data_id:
            return user_profile['data'][packed_data_txt]

    return None


def get_mutable_data_profile_ex(user_profile, data_id):
    """
    Get the metadata for a piece of mutable data, given
    the user's profile and data_id.
    Return the (route (as a dict), version, metadata key) on success
    Return (None, None, None) if not found
    """

    if 'data' not in user_profile:
        return None, None, None

    for packed_data_txt in user_profile['data']:
        if not is_mutable_data_md(packed_data_txt):
            continue

        unpacked_data_id, version = unpack_mutable_data_md(packed_data_txt)
        if data_id == unpacked_data_id:
            return user_profile['data'][packed_data_txt], version, packed_data_txt

    return None, None, None


def get_mutable_data_profile_md(user_profile, data_id):
    """
    Get the serialized profile key for a piece of mutable data, given
    the user's profile and data_id.
    Return the mutable data key on success (an opaque but unique string)
    Return None if not found
    """

    if 'data' not in user_profile:
        return None

    for packed_data_txt in user_profile['data']:
        if not is_mutable_data_md(packed_data_txt):
            continue

        unpacked_data_id, version = None, None
        try:
            unpacked_data_id, version = unpack_mutable_data_md(packed_data_txt)
        except:
            # TODO: Check if we should use more fine-grained exception handling
            continue

        if data_id == unpacked_data_id:
            return packed_data_txt

    return None


def list_mutable_data(user_profile):
    """
    Get a list of all mutable data information.
    Return [(data_id, version)]
    """
    if 'data' not in user_profile:
        return []

    ret = []
    for packed_data_txt in user_profile['data']:
        if not is_mutable_data_md(packed_data_txt):
            continue

        unpacked_data_id, version = None, None
        try:
            unpacked_data_id, version = unpack_mutable_data_md(packed_data_txt)
        except:
            # TODO: Check if we should use more fine-grained exception handling
            continue

        ret.append((unpacked_data_id, version))

    return ret


def put_mutable_data_profile(user_profile, data_id, version, zonefile):
    """
    Put mutable data to a user's profile.
    Only works if the zonefile has a later version field, or doesn't exist.
    Return True on success
    Return False if this is a duplicate
    Raise an Exception if the route is invalid, or if this is a duplicate zonefile.
    """

    if not user_profile.setdefault('data', {}):
        user_profile['data'].update(zonefile)
        return True

    existing_info, existing_version, existing_md = get_mutable_data_profile_ex(user_profile, data_id)

    if existing_info is None:
        # first case of this mutable datum
        user_profile['data'].update(zonefile)
        return True

    # must be a newer version
    if existing_version >= version:
        msg = 'Will not put mutable data zonefile; existing version {} >= {}'
        log.debug(msg.format(existing_version, version))
        return False

    # replace
    del user_profile['data'][existing_md]
    user_profile['data'].update(zonefile)

    return True


def remove_mutable_data_profile(user_profile, data_id):
    """
    Remove info for mutable data from a user profile.
    Return True if removed
    Return False if the user had no such data.
    """

    if 'data' not in user_profile:
        return False

    # check for it
    for packed_data_txt in user_profile['data']:
        if not is_mutable_data_md(packed_data_txt):
            continue

        unpacked_data_id, version = unpack_mutable_data_md(packed_data_txt)
        if data_id == unpacked_data_id:
            del user_profile['data'][packed_data_txt]
            return True

    # already gone
    return False


def pack_mutable_data_md(data_id, version):
    """
    Pack an mutable datum's metadata into a string
    """
    return 'mutable:{}:{}'.format(base64.b64encode(data_id.encode('utf-8')), version)


def pack_mutable_data_md_prefix(data_id):
    """
    Pack an mutable datum's metadata prefix into a string (i.e. skip the version)
    """
    return 'mutable:{}:'.format(base64.b64encode(data_id.encode('utf-8')))


def unpack_mutable_data_md(rec):
    """
    Unpack an mutable datum's key into its metadata
    """
    parts = rec.split(':')
    assert len(parts) == 3, 'parts = {}'.format(parts)
    assert parts[0] == 'mutable', 'parts = {}'.format(parts)

    data_id = base64.b64decode(parts[1])
    version = int(parts[2])

    return data_id, version


def is_mutable_data_md(rec):
    """
    Is this a mutable datum's key?
    """
    parts = rec.split(':')
    if len(parts) != 3:
        return False

    return parts[0] == 'mutable'


def make_mutable_data(data_id, version, urls):
    """
    Make a zonefile for mutable data.
    """
    data_name = pack_mutable_data_md(data_id, version)

    uris = [url_to_uri_record(url, data_id) for url in urls]

    return {data_name: {'uri': uris}}


def mutable_data_version(user_profile, data_id):
    """
    Get the data version for a piece of mutable data.
    Return 0 if it doesn't exist
    """

    key = get_mutable_data_profile_md(user_profile, data_id)
    if key is None:
        log.debug('No mutable data zonefiles installed for "{}"'.format(data_id))
        return 0

    data_id, version = unpack_mutable_data_md(key)

    return version


def mutable_data_urls(mutable_info):
    """
    Get the URLs from a mutable data zonefile
    """
    uri_records = mutable_info.get('uri')

    if uri_records is None:
        return None

    return [u['target'].strip('"') for u in uri_records]
