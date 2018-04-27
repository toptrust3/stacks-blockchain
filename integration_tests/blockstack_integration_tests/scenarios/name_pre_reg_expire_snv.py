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

import testlib
import virtualchain
import json
import blockstack 

wallets = [
    testlib.Wallet( "5JesPiN68qt44Hc2nT8qmyZ1JDwHebfoh9KQ52Lazb1m1LaKNj9", 100000000000 ),
    testlib.Wallet( "5KHqsiU9qa77frZb6hQy9ocV7Sus9RWJcQGYYBJJBb2Efj1o77e", 100000000000 ),
    testlib.Wallet( "5Kg5kJbQHvk1B64rJniEmgbD83FpZpbw2RjdAZEzTefs9ihN3Bz", 100000000000 ),
    testlib.Wallet( "5JuVsoS9NauksSkqEjbUZxWwgGDQbMwPsEfoRBSpLpgDX1RtLX7", 100000000000 ),
    testlib.Wallet( "5KEpiSRr1BrT8vRD7LKGCEmudokTh1iMHbiThMQpLdwBwhDJB1T", 100000000000 )
]

consensus = "17ac43c1d8549c3181b200f1bf97eb7d"
snv_block_id = None
last_consensus = None

# do the 2nd epoch
NAMESPACE_LIFETIME_MULTIPLIER = blockstack.get_epoch_namespace_lifetime_multiplier( blockstack.EPOCH_1_END_BLOCK + 1, "test" )

def scenario( wallets, **kw ):

    global snv_block_id, last_consensus

    testlib.blockstack_namespace_preorder( "test", wallets[1].addr, wallets[0].privkey )
    testlib.next_block( **kw )

    # NOTE: names expire in 5 * NAMESPACE_LIFETIME_MULTIPLIER blocks
    testlib.blockstack_namespace_reveal( "test", wallets[1].addr, 5, 250, 4, [6,5,4,3,2,1,0,0,0,0,0,0,0,0,0,0], 10, 10, wallets[0].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_namespace_ready( "test", wallets[1].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_name_preorder( "foo.test", wallets[2].privkey, wallets[3].addr )
    testlib.next_block( **kw )

    testlib.blockstack_name_register( "foo.test", wallets[2].privkey, wallets[3].addr )
    testlib.next_block( **kw )

    snv_block_id = testlib.get_current_block()
    
    for i in xrange(0, 5 * NAMESPACE_LIFETIME_MULTIPLIER):
        testlib.next_block( **kw )
    
    # this one expires the name
    testlib.next_block( **kw )

    last_consensus = testlib.get_consensus_at( testlib.get_current_block() )


def check( state_engine ):

    global snv_block_id, last_consensus 

    # not revealed, but ready 
    ns = state_engine.get_namespace_reveal( "test" )
    if ns is not None:
        return False 

    ns = state_engine.get_namespace( "test" )
    if ns is None:
        return False 

    if ns['namespace_id'] != 'test':
        return False 

    # not preordered
    preorder = state_engine.get_name_preorder( "foo.test", virtualchain.make_payment_script(wallets[2].addr), wallets[3].addr )
    if preorder is not None:
        return False
    
    # not registered 
    name_rec = state_engine.get_name( "foo.test" )
    if name_rec is not None:
        print "Not expired:"
        print json.dumps( name_rec, indent=4 )
        return False 

    #  snv lookup works
    snv_rec = blockstack.lib.snv.snv_lookup( "foo.test", snv_block_id, last_consensus)
    if 'error' in snv_rec:
        print json.dumps(snv_rec, indent=4 )
        print "Expected (%s, %s) from (%s, %s)" % (snv_block_id, snv_consensus, lastblock, lastconsensus)
        return False

    print snv_rec 
    return True 
