#!/usr/bin/env python2
# -*- coding: utf-8 -*-
"""
    Blockstack
    ~~~~~
    copyright: (c) 2014-2015 by Halfmoon Labs, Inc.
    copyright: (c) 2016 by Blockstack.org

    This file is part of Blockstack

    Blockstack is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    Blockstack is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.
    You should have received a copy of the GNU General Public License
    along with Blockstack. If not, see <http://www.gnu.org/licenses/>.
"""

import preorder
import register
import transfer
import update
import revoke
import nameimport
import namespacepreorder
import namespacereveal
import namespaceready
import announce

import binascii
import copy

from ..nameset import CONSENSUS_FIELDS_REQUIRED, NAMEREC_MUTATE_FIELDS  # , NAMEREC_BACKUP_FIELDS
from ..config import *

from .register import get_registration_recipient_from_outputs 
from .transfer import get_transfer_recipient_from_outputs
from .nameimport import get_import_update_hash_from_outputs

from .preorder import tx_extract as extract_preorder, check as check_preorder
from .register import tx_extract as extract_registration, check_register as check_registration, check_renewal
from .transfer import tx_extract as extract_transfer, check as check_transfer, canonicalize as canonicalize_transfer
from .update import tx_extract as extract_update, check as check_update, canonicalize as canonicalize_update
from .revoke import tx_extract as extract_revoke, check as check_revoke
from .nameimport import tx_extract as extract_name_import, check as check_name_import, canonicalize as canonicalize_name_import
from .namespacepreorder import tx_extract as extract_namespace_preorder, check as check_namespace_preorder, canonicalize as canonicalize_namespace_preorder, decanonicalize as decanonicalize_namespace_preorder
from .namespacereveal import tx_extract as extract_namespace_reveal, check as check_namespace_reveal, canonicalize as canonicalize_namespace_reveal, decanonicalize as decanonicalize_namespace_reveal
from .namespaceready import tx_extract as extract_namespace_ready, check as check_namespace_ready, canonicalize as canonicalize_namespace_ready, decanonicalize as decanonicalize_namespace_ready
from .announce import tx_extract as extract_announce, check as check_announce

SERIALIZE_FIELDS = {
    "NAME_PREORDER": preorder.FIELDS,
    "NAME_REGISTRATION": register.FIELDS,
    "NAME_RENEWAL": register.FIELDS,
    "NAME_UPDATE": update.FIELDS,
    "NAME_TRANSFER": transfer.FIELDS,
    "NAME_REVOKE": revoke.FIELDS,
    "NAME_IMPORT": nameimport.FIELDS,
    "NAMESPACE_PREORDER": namespacepreorder.FIELDS,
    "NAMESPACE_REVEAL": namespacereveal.FIELDS,
    "NAMESPACE_READY": namespaceready.FIELDS,
    "ANNOUNCE": announce.FIELDS
}

MUTATE_FIELDS = {
    "NAME_PREORDER": preorder.MUTATE_FIELDS,
    "NAME_REGISTRATION": register.REGISTER_MUTATE_FIELDS,
    "NAME_RENEWAL": register.RENEWAL_MUTATE_FIELDS,
    "NAME_UPDATE": update.MUTATE_FIELDS,
    "NAME_TRANSFER": transfer.MUTATE_FIELDS,
    "NAME_REVOKE": revoke.MUTATE_FIELDS,
    "NAME_IMPORT": nameimport.MUTATE_FIELDS,
    "NAMESPACE_PREORDER": namespacepreorder.MUTATE_FIELDS,
    "NAMESPACE_REVEAL": namespacereveal.MUTATE_FIELDS,
    "NAMESPACE_READY": namespaceready.MUTATE_FIELDS,
    "ANNOUNCE": announce.MUTATE_FIELDS
}

# fields that do not have columns in the db schema, but are part of this operation's consensus ops hash
UNSTORED_CANONICAL_FIELDS = {
    'NAME_PREORDER': [],
    'NAME_REGISTRATION': [],
    'NAME_RENEWAL': [],
    'NAME_UPDATE': update.UNSTORED_CANONICAL_FIELDS,
    'NAME_TRANSFER': transfer.UNSTORED_CANONICAL_FIELDS,
    'NAME_REVOKE': [],
    'NAME_IMPORT': [],
    'NAMESPACE_PREORDER': [],
    'NAMESPACE_REVEAL': [],
    'NAMESPACE_READY': [],
    'ANNOUNCE': []
}

# NOTE: these all have the same signatures
EXTRACT_METHODS = {
    "NAME_PREORDER": extract_preorder,
    "NAME_REGISTRATION": extract_registration,
    "NAME_RENEWAL": extract_registration,
    "NAME_UPDATE": extract_update,
    "NAME_TRANSFER": extract_transfer,
    "NAME_REVOKE": extract_revoke,
    "NAME_IMPORT": extract_name_import,
    "NAMESPACE_PREORDER": extract_namespace_preorder,
    "NAMESPACE_REVEAL": extract_namespace_reveal,
    "NAMESPACE_READY": extract_namespace_ready,
    "ANNOUNCE": extract_announce
}

# NOTE: these all have the same signature
CHECK_METHODS = {
    "NAME_PREORDER": check_preorder,
    "NAME_REGISTRATION": check_registration,
    "NAME_RENEWAL": check_renewal,
    "NAME_UPDATE": check_update,
    "NAME_TRANSFER": check_transfer,
    "NAME_REVOKE": check_revoke,
    "NAME_IMPORT": check_name_import,
    "NAMESPACE_PREORDER": check_namespace_preorder,
    "NAMESPACE_REVEAL": check_namespace_reveal,
    "NAMESPACE_READY": check_namespace_ready,
    "ANNOUNCE": check_announce
}

# NOTE: these all have the same signature
CANONICALIZE_METHODS = {
    "NAME_UPDATE": canonicalize_update,
    "NAME_TRANSFER": canonicalize_transfer,
    "NAME_IMPORT": canonicalize_name_import,
    "NAMESPACE_PREORDER": canonicalize_namespace_preorder,
    "NAMESPACE_REVEAL": canonicalize_namespace_reveal,
    "NAMESPACE_READY": canonicalize_namespace_ready,
}


# NOTE: these all have the same signature
DECANONICALIZE_METHODS = {
    "NAMESPACE_PREORDER": decanonicalize_namespace_preorder,
    "NAMESPACE_REVEAL": decanonicalize_namespace_reveal,
    "NAMESPACE_READY": decanonicalize_namespace_ready
}


# build-in sanity checks....
# required consensus fields are required!
for opcode, serialize_set in SERIALIZE_FIELDS.items():
    if len(serialize_set) == 0:
        continue

    for required_consensus_field in CONSENSUS_FIELDS_REQUIRED:
        if required_consensus_field not in serialize_set:
            # do not even allow this package to be imported 
            raise Exception("BUG: missing required consensus field '%s' in '%s' definition" % (required_consensus_field, opcode))

# required mutate fields must be present 
for opcode, mutate_set in MUTATE_FIELDS.items():
    if len(mutate_set) == 0:
        continue 

    for required_mutate_field in NAMEREC_MUTATE_FIELDS:
        if required_mutate_field not in mutate_set:
            # do not even allow this package to be imported 
            raise Exception("BUG: missing required mutate field '%s' of '%s' definition" % (required_mutate_field, opcode))

del opcode
del mutate_set
del serialize_set
del required_mutate_field
del required_consensus_field


def op_extract(op_name, data, senders, inputs, outputs, block_id, vtxindex, txid):
    """
    Extract an operation from transaction data.
    Return the extracted fields as a dict.
    """
    global EXTRACT_METHODS

    if op_name not in EXTRACT_METHODS.keys():
        raise Exception("No such operation '%s'" % op_name)

    method = EXTRACT_METHODS[op_name]
    op_data = method( data, senders, inputs, outputs, block_id, vtxindex, txid )
    return op_data


def op_canonicalize(op_name, parsed_op):
    """
    Get the canonical representation of a parsed operation's data.
    Meant for backwards-compatibility
    """
    global CANONICALIZE_METHODS

    if op_name not in CANONICALIZE_METHODS:
        # no canonicalization needed
        return parsed_op
    else:
        return CANONICALIZE_METHODS[op_name](parsed_op)


def op_decanonicalize(op_name, canonical_op):
    """
    Get the current representation of a parsed operation's data, given the canonical representation
    Meant for backwards-compatibility
    """
    global DECANONICALIZE_METHODS

    if op_name not in DECANONICALIZE_METHODS:
        # no decanonicalization needed
        return canonical_op
    else:
        return DECANONICALIZE_METHODS[op_name](canonical_op)


def op_check( state_engine, nameop, block_id, checked_ops ):
    """
    Given the state engine, the current block, the list of pending
    operations processed so far, and the current operation, determine
    whether or not it should be accepted.

    The operation is allowed to be "type-cast" to a new operation, but only once.
    If this happens, the operation will be checked again.
    Subsequent casts are considered bugs, and will cause a program abort.

    TODO: remove type-cast
    TODO: remove op_check_quirks
    """

    global CHECK_METHODS, MUTATE_FIELDS

    count = 0
    while count < 3:

        count += 1

        nameop_clone = copy.deepcopy( nameop )
        opcode = None

        if 'opcode' not in nameop_clone.keys():
            op = nameop_clone.get('op', None)
            try:
                assert op is not None, "BUG: no op defined"
                opcode = op_get_opcode_name( op )
                assert opcode is not None, "BUG: op '%s' undefined" % op
            except Exception, e:
                log.exception(e)
                log.error("FATAL: BUG: no 'op' defined")
                sys.exit(1)

        else:
            opcode = nameop_clone['opcode']
  
        check_method = CHECK_METHODS.get( opcode, None )
        try:
            assert check_method is not None, "BUG: no check-method for '%s'" % opcode
        except Exception, e:
            log.exception(e)
            log.error("FATAL: BUG: no check-method for '%s'" % opcode )
            sys.exit(1)

        rc = check_method( state_engine, nameop_clone, block_id, checked_ops )
        if not rc:
            # rejected
            break

        # was this type-cast to a new operation?
        new_opcode = nameop_clone.get('opcode', None)
        if new_opcode is None or new_opcode == opcode:
            # we're done
            nameop.clear()
            nameop.update( nameop_clone )
            break

        else:
            # try again 
            log.debug("Nameop re-interpreted from '%s' to '%s' (%s)" % (opcode, new_opcode, count))
            nameop['opcode'] = new_opcode 
            continue

    try:
        assert count < 3, "BUG: multiple opcode type-casts detected"
    except Exception, e:
        log.exception(e)
        log.error("FATAL: BUG: multiple opcode type-casts detected")
        sys.exit(1)
    
    if rc:
        nameop = op_canonicalize(nameop['opcode'], nameop)

        # make sure we don't send unstored fields to the db that are otherwise canonical
        unstored_canonical_fields = UNSTORED_CANONICAL_FIELDS.get(nameop['opcode'])
        assert unstored_canonical_fields is not None, "BUG: no UNSTORED_CANONICAL_FIELDS entry for {}".format(nameop['opcode'])

        for f in unstored_canonical_fields:
            if f in nameop:
                del nameop[f]

        # op_check_quirks( state_engine, nameop, block_id, checked_ops )

    return rc


def op_get_mutate_fields( op_name ):
    """
    Get the names of the fields that will change
    when this operation gets applied to a record.
    """

    global MUTATE_FIELDS

    if op_name not in MUTATE_FIELDS.keys():
        raise Exception("No such operation '%s'" % op_name)

    fields = MUTATE_FIELDS[op_name][:]
    return fields


def op_get_consensus_fields( op_name ):
    """
    Get the set of consensus-generating fields for an operation.
    """

    global SERIALIZE_FIELDS
    
    if op_name not in SERIALIZE_FIELDS.keys():
        raise Exception("No such operation '%s'" % op_name )

    fields = SERIALIZE_FIELDS[op_name][:]
    return fields

