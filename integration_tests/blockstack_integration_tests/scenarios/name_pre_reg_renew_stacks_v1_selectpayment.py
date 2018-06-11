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

# activate tokens
"""
TEST ENV BLOCKSTACK_EPOCH_1_END_BLOCK 682
TEST ENV BLOCKSTACK_EPOCH_2_END_BLOCK 683
TEST ENV BLOCKSTACK_EPOCH_3_END_BLOCK 684
TEST ENV BLOCKSTACK_EPOCH_2_NAMESPACE_LIFETIME_MULTIPLIER 1
TEST ENV BLOCKSTACK_EPOCH_3_NAMESPACE_LIFETIME_MULTIPLIER 1
"""

wallets = [
    testlib.Wallet( "5JesPiN68qt44Hc2nT8qmyZ1JDwHebfoh9KQ52Lazb1m1LaKNj9", 100000000000 ),
    testlib.Wallet( "5KHqsiU9qa77frZb6hQy9ocV7Sus9RWJcQGYYBJJBb2Efj1o77e", 100000000000 ),
    testlib.Wallet( "5Kg5kJbQHvk1B64rJniEmgbD83FpZpbw2RjdAZEzTefs9ihN3Bz", 100000000000 ),
    testlib.Wallet( "5JuVsoS9NauksSkqEjbUZxWwgGDQbMwPsEfoRBSpLpgDX1RtLX7", 100000000000 ),
    testlib.Wallet( "5KEpiSRr1BrT8vRD7LKGCEmudokTh1iMHbiThMQpLdwBwhDJB1T", 100000000000 )
]

consensus = "17ac43c1d8549c3181b200f1bf97eb7d"
pk = None
pk2 = None

def scenario( wallets, **kw ):
    global pk, pk2

    testlib.blockstack_namespace_preorder( "test", wallets[1].addr, wallets[0].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_namespace_reveal( "test", wallets[1].addr, 52595, 250, 4, [6,6,6,6,6,6,0,0,0,0,0,0,0,0,0,0], 10, 10, wallets[0].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_namespace_ready( "test", wallets[1].privkey )
    testlib.next_block( **kw )

    # pay for a name in a v1 namespace with Stacks
    pk = virtualchain.lib.ecdsalib.ecdsa_private_key().to_hex()
    pk2 = virtualchain.lib.ecdsalib.ecdsa_private_key().to_hex()
    addr = virtualchain.address_reencode(virtualchain.get_privkey_address(pk))
    addr2 = virtualchain.address_reencode(virtualchain.get_privkey_address(pk2))

    # calculate the cost of doing so
    namespace = testlib.get_state_engine().get_namespace('test')
    stacks_price = blockstack.lib.scripts.price_name_stacks('baz', namespace, testlib.get_current_block(**kw))
    btc_price = blockstack.lib.scripts.price_name('baz', namespace, testlib.get_current_block(**kw))

    print ''
    print 'price of {} in Stacks is {}'.format('baz.test', stacks_price)
    print 'price of {} in BTC is {}'.format('baz.test', btc_price)
    print ''

    testlib.blockstack_send_tokens(addr, "STACKS", stacks_price, wallets[0].privkey)
    testlib.blockstack_send_tokens(addr2, "STACKS", stacks_price * 2, wallets[0].privkey)
    testlib.send_funds(wallets[0].privkey, 10*btc_price, addr)
    testlib.send_funds(wallets[0].privkey, 10*btc_price, addr2)
    testlib.next_block(**kw)

    # preorder/register using Stacks
    testlib.blockstack_name_preorder( "baz.test", wallets[2].privkey, addr2, price={'units': 'STACKS', 'amount': stacks_price})
    testlib.blockstack_name_preorder( "goo.test", wallets[2].privkey, addr2, price={'units': 'STACKS', 'amount': stacks_price})
    testlib.next_block( **kw )

    testlib.blockstack_name_register( "baz.test", wallets[2].privkey, addr2 )
    testlib.blockstack_name_register( "goo.test", wallets[2].privkey, addr2 )
    testlib.next_block( **kw )

    balance_before = testlib.get_addr_balances(addr2)[addr2]['STACKS']

    # pay with both Stacks and Bitcoin
    # should favor Stacks payment over Bitcoin payment if we pay enough stacks.
    # Stacks should have been burned, as well as BTC.
    res = testlib.blockstack_name_renew('baz.test', pk2, price={'units': 'STACKS', 'amount': stacks_price}, tx_only=True, expect_success=True)
    txhex = res['transaction']
    tx = virtualchain.btc_tx_deserialize(txhex)

    # up the burn amount 
    btc_price = blockstack.lib.scripts.price_name('baz', namespace, testlib.get_current_block(**kw))
    tx['outs'][3]['script'] = virtualchain.btc_make_payment_script(blockstack.lib.config.BLOCKSTACK_BURN_ADDRESS)
    tx['outs'][3]['value'] = btc_price

    tx['outs'][4]['value'] -= btc_price

    # re-sign 
    for i in tx['ins']:
        i['script'] = ''

    txhex = virtualchain.btc_tx_serialize(tx)
    txhex_signed = virtualchain.tx_sign_all_unsigned_inputs(pk2, testlib.get_utxos(addr2), txhex)

    # re-sign the last output with the payment key
    tx_signed = virtualchain.btc_tx_deserialize(txhex_signed)
    tx_signed['ins'][-1]['script'] = ''
    txhex_signed = virtualchain.tx_sign_all_unsigned_inputs(testlib.get_default_payment_wallet().privkey, testlib.get_utxos(testlib.get_default_payment_wallet().addr), virtualchain.btc_tx_serialize(tx_signed))
    
    print txhex_signed

    res = testlib.broadcast_transaction(txhex_signed)
    if 'error' in res:
        print res
        return False

    testlib.next_block(**kw)

    # should have paid in Stacks
    balance_after = testlib.get_addr_balances(addr2)[addr2]['STACKS']
    if balance_after != balance_before - stacks_price:
        print 'baz.test cost {}'.format(balance_before - balance_after)
        return False

    balance_before = testlib.get_addr_balances(addr2)[addr2]['STACKS']

    # register a name where we pay not enough stacks, but enough bitcoin.  should still go through
    # should favor Bitcoin payment over Stacks payment.
    # No Stacks should be burned on NAME_RENEWAL, since it's clear that we burned Bitcoin from the transaction.
    res = testlib.blockstack_name_renew('goo.test', pk2, price={'units': 'STACKS', 'amount': stacks_price-1}, tx_only=True, expect_success=True)
    txhex = res['transaction']
    tx = virtualchain.btc_tx_deserialize(txhex)

    # up the burn amount to the name price
    btc_price = blockstack.lib.scripts.price_name('goo', namespace, testlib.get_current_block(**kw))
    tx['outs'][3]['script'] = virtualchain.btc_make_payment_script(blockstack.lib.config.BLOCKSTACK_BURN_ADDRESS)
    tx['outs'][3]['value'] = btc_price

    tx['outs'][4]['value'] -= btc_price

    # re-sign 
    for i in tx['ins']:
        i['script'] = ''

    txhex = virtualchain.btc_tx_serialize(tx)
    txhex_signed = virtualchain.tx_sign_all_unsigned_inputs(pk2, testlib.get_utxos(addr2), txhex)

    # re-sign the last output with the payment key
    tx_signed = virtualchain.btc_tx_deserialize(txhex_signed)
    tx_signed['ins'][-1]['script'] = ''
    txhex_signed = virtualchain.tx_sign_all_unsigned_inputs(testlib.get_default_payment_wallet().privkey, testlib.get_utxos(testlib.get_default_payment_wallet().addr), virtualchain.btc_tx_serialize(tx_signed))
    
    print txhex_signed

    res = testlib.broadcast_transaction(txhex_signed)
    if 'error' in res:
        print res
        return False

    testlib.next_block(**kw)

    # should have paid in Stacks
    balance_after = testlib.get_addr_balances(addr2)[addr2]['STACKS']
    if balance_after != balance_before - stacks_price + 1:
        print 'goo.test paid {}'.format(balance_before - balance_after)
        return False


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

    for name in ['baz.test', 'goo.test']:
        # not preordered
        addr2 = virtualchain.address_reencode(virtualchain.get_privkey_address(pk2))
        preorder = state_engine.get_name_preorder( name, virtualchain.make_payment_script(wallets[2].addr), addr2 )
        if preorder is not None:
            print "preorder exists"
            return False
        
        # registered 
        name_rec = state_engine.get_name( name )
        if name_rec is None:
            print "name does not exist"
            return False 

        # owned by
        if name_rec['address'] != addr2 or name_rec['sender'] != virtualchain.make_payment_script(addr2):
            print "sender is wrong"
            return False 

    # paid for baz.test with Stacks, but nevertheless burned Bitcoin
    # however, baz.test's burn output is equal to the bitcoin price
    for name in ['baz']:
        name_rec = state_engine.get_name( name + '.test' )
        stacks_price = blockstack.lib.scripts.price_name_stacks(name, ns, state_engine.lastblock)
        if name_rec['token_fee'] != stacks_price:
            print 'paid wrong token fee for {}.test'.format(name)
            print 'expected {} ({}), got {} ({})'.format(stacks_price, type(stacks_price), name_rec['token_fee'], type(name_rec['token_fee']))
            return False

        if name_rec['op_fee'] != blockstack.lib.scripts.price_name(name, ns, state_engine.lastblock):
            print 'paid wrong BTC for baz.test ({})'.format(name_rec['op_fee'])
            return False

    for name in ['goo']:
        name_rec = state_engine.get_name( name + '.test' )

        # paid both BTC and tokens
        if name_rec['token_fee'] != blockstack.lib.scripts.price_name_stacks(name, ns, state_engine.lastblock) - 1:
            print 'paid wrong token fee for {}.test'.format(name)
            return False

        if name_rec['op_fee'] != blockstack.lib.scripts.price_name(name, ns, state_engine.lastblock):
            print 'paid wrong BTC for {}.test ({})'.format(name, name_rec['op_fee'])
            return False

    return True
