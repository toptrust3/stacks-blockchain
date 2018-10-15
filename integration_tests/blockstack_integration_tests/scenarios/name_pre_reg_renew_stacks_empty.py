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
import blockstack
import json

# activate tokens
"""
TEST ENV BLOCKSTACK_EPOCH_1_END_BLOCK 682
TEST ENV BLOCKSTACK_EPOCH_2_END_BLOCK 683
TEST ENV BLOCKSTACK_EPOCH_3_END_BLOCK 684
TEST ENV BLOCKSTACK_EPOCH_2_NAMESPACE_LIFETIME_MULTIPLIER 1
TEST ENV BLOCKSTACK_EPOCH_3_NAMESPACE_LIFETIME_MULTIPLIER 1
"""

wallets = [
    testlib.Wallet( "5JesPiN68qt44Hc2nT8qmyZ1JDwHebfoh9KQ52Lazb1m1LaKNj9", 0 ),
    testlib.Wallet( "5KHqsiU9qa77frZb6hQy9ocV7Sus9RWJcQGYYBJJBb2Efj1o77e", 100000000000 ),
    testlib.Wallet( "5Kg5kJbQHvk1B64rJniEmgbD83FpZpbw2RjdAZEzTefs9ihN3Bz", 100000000000 ),
    testlib.Wallet( "5JuVsoS9NauksSkqEjbUZxWwgGDQbMwPsEfoRBSpLpgDX1RtLX7", 100000000000 ),
    testlib.Wallet( "5KEpiSRr1BrT8vRD7LKGCEmudokTh1iMHbiThMQpLdwBwhDJB1T", 100000000000 )
]

consensus = "17ac43c1d8549c3181b200f1bf97eb7d"

def scenario( wallets, **kw ):

    # should fail
    testlib.blockstack_namespace_preorder( "test_fail", wallets[1].addr, wallets[0].privkey, safety_checks=False, price={'units': 'STACKS', 'amount': 0} )
    testlib.next_block( **kw )
    
    testlib.blockstack_namespace_reveal( "test_fail", wallets[1].addr, 52595, 250, 4, [6,5,4,3,2,1,0,0,0,0,0,0,0,0,0,0], 10, 10, wallets[0].privkey, version_bits=blockstack.NAMESPACE_VERSION_PAY_WITH_STACKS )
    testlib.next_block( **kw )
    testlib.expect_snv_fail_at('test_fail', testlib.get_current_block(**kw))

    # should succeed
    testlib.blockstack_namespace_preorder( "test", wallets[1].addr, wallets[3].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_namespace_reveal( "test", wallets[1].addr, 52595, 250, 4, [6,5,4,3,2,1,0,0,0,0,0,0,0,0,0,0], 10, 10, wallets[3].privkey, version_bits=blockstack.NAMESPACE_VERSION_PAY_WITH_STACKS )
    testlib.next_block( **kw )

    testlib.blockstack_namespace_ready( "test", wallets[1].privkey )
    testlib.next_block( **kw )
   
    # should fail (empty balance) 
    testlib.blockstack_name_preorder('foo_fail.test', wallets[0].privkey, wallets[3].addr, safety_checks=False, expect_fail=True)
    testlib.next_block(**kw)

    testlib.blockstack_name_register('foo_fail.test', wallets[0].privkey, wallets[3].addr, safety_checks=False)
    testlib.next_block(**kw)
    testlib.expect_snv_fail_at('foo_fail.test', testlib.get_current_block(**kw))

    # should succeed
    testlib.blockstack_name_preorder( "foo.test", wallets[2].privkey, wallets[3].addr )
    testlib.next_block( **kw )

    testlib.blockstack_name_register( "foo.test", wallets[2].privkey, wallets[3].addr )
    testlib.next_block( **kw )

    # deplete balance 
    balance = json.loads(testlib.nodejs_cli('balance', wallets[2].addr))
    testlib.blockstack_send_tokens(wallets[0].addr, 'STACKS', int(balance['STACKS']), wallets[2].privkey)

    balance = json.loads(testlib.nodejs_cli('balance', wallets[3].addr))
    testlib.blockstack_send_tokens(wallets[0].addr, 'STACKS', int(balance['STACKS']), wallets[3].privkey)

    testlib.next_block(**kw)

    # should fail--not enough funds
    testlib.blockstack_name_renew( "foo.test", wallets[3].privkey, expect_fail=True )
    testlib.next_block( **kw )
    testlib.expect_snv_fail_at('foo.test', testlib.get_current_block(**kw))

    # should fail--not enough funds
    testlib.blockstack_name_preorder( "baz.test", wallets[2].privkey, wallets[3].addr, expect_fail=True, safety_checks=False )
    testlib.next_block( **kw )
    testlib.expect_snv_fail_at('baz.test', testlib.get_current_block(**kw))

    testlib.blockstack_name_register( "baz.test", wallets[2].privkey, wallets[3].addr, safety_checks=False )
    testlib.next_block( **kw )
    testlib.expect_snv_fail_at('baz.test', testlib.get_current_block(**kw))


def check( state_engine ):

    # not revealed, but ready 
    ns = state_engine.get_namespace_reveal( "test" )
    if ns is not None:
        print "namespace reveal exists"
        return False 

    ns = state_engine.get_namespace( "test" )
    if ns is None:
        print "no namespace"
        return False 

    if ns['namespace_id'] != 'test':
        print "wrong namespace"
        return False 

    # no failed namespace 
    ns = state_engine.get_namespace_reveal('test_fail')
    if ns is not None:
        print 'namespace reveal exists for failed namespace'
        return False

    ns = state_engine.get_namespace('test_fail')
    if ns is not None:
        print 'failed namespace exists'
        return False

    # not preordered
    preorder = state_engine.get_name_preorder( "foo.test", virtualchain.make_payment_script(wallets[2].addr), wallets[3].addr )
    if preorder is not None:
        print "preorder exists"
        return False
    
    preorder = state_engine.get_name_preorder( "foo_fail.test", virtualchain.make_payment_script(wallets[4].addr), wallets[3].addr )
    if preorder is not None:
        print "preorder exists"
        return False

    # registered 
    name_rec = state_engine.get_name( "foo.test" )
    if name_rec is None:
        print "name does not exist"
        return False 

    # owned by
    if name_rec['address'] != wallets[3].addr or name_rec['sender'] != virtualchain.make_payment_script(wallets[3].addr):
        print "sender is wrong"
        return False

    name_rec = state_engine.get_name( "foo_fail.test" )
    if name_rec is not None:
        print "name accidentally exists"
        return False 

    name_rec = state_engine.get_name( "baz.test" )
    if name_rec is not None:
        print "name accidentally exists"
        return False 

    return True
