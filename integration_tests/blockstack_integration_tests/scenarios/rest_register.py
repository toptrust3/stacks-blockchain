#!/usr/bin/env python
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
import os
import testlib
import pybitcoin
import urllib2
import json
import blockstack_client
import blockstack_profiles
import blockstack_gpg
import sys
import keylib
import time

from keylib import ECPrivateKey, ECPublicKey

wallets = [
    testlib.Wallet( "5JesPiN68qt44Hc2nT8qmyZ1JDwHebfoh9KQ52Lazb1m1LaKNj9", 100000000000 ),
    testlib.Wallet( "5KHqsiU9qa77frZb6hQy9ocV7Sus9RWJcQGYYBJJBb2Efj1o77e", 100000000000 ),
    testlib.Wallet( "5Kg5kJbQHvk1B64rJniEmgbD83FpZpbw2RjdAZEzTefs9ihN3Bz", 100000000000 ),
    testlib.Wallet( "5JuVsoS9NauksSkqEjbUZxWwgGDQbMwPsEfoRBSpLpgDX1RtLX7", 100000000000 ),
    testlib.Wallet( "5KEpiSRr1BrT8vRD7LKGCEmudokTh1iMHbiThMQpLdwBwhDJB1T", 100000000000 ),
    testlib.Wallet( "5K5hDuynZ6EQrZ4efrchCwy6DLhdsEzuJtTDAf3hqdsCKbxfoeD", 100000000000 ),
    testlib.Wallet( "5J39aXEeHh9LwfQ4Gy5Vieo7sbqiUMBXkPH7SaMHixJhSSBpAqz", 100000000000 ),
    testlib.Wallet( "5K9LmMQskQ9jP1p7dyieLDAeB6vsAj4GK8dmGNJAXS1qHDqnWhP", 100000000000 ),
    testlib.Wallet( "5KcNen67ERBuvz2f649t9F2o1ddTjC5pVUEqcMtbxNgHqgxG2gZ", 100000000000 ),
    testlib.Wallet( "5KMbNjgZt29V6VNbcAmebaUT2CZMxqSridtM46jv4NkKTP8DHdV", 100000000000 ),
]

consensus = "17ac43c1d8549c3181b200f1bf97eb7d"
wallet_keys = None
wallet_keys_2 = None
error = False

index_file_data = "<html><head></head><body>foo.test hello world</body></html>"
resource_data = "hello world"

def verify_in_queue( ses, name, queue_name, tx_hash, expected_length=1 ):

    # verify that it's in the queue 
    res = testlib.blockstack_REST_call('GET', '/api/v1/blockchains/bitcoin/pending', ses, name='foo.test', appname='register')
    if 'error' in res:
        res['test'] = 'Failed to get queues'
        print json.dumps(res)
        error = True
        return False

    res = res['response']

    # needs to be in the queue 
    if not res.has_key('queues'):
        res['test'] = 'Missing queues'
        print json.dumps(res)
        error = True
        return False

    if not res['queues'].has_key(queue_name):
        res['test'] = 'Missing {} queue'.format(queue_name)
        print json.dumps(res)
        error = True
        return False
    
    if len(res['queues'][queue_name]) != expected_length:
        res['test'] = 'invalid preorder queue'
        print json.dumps(res)
        error = True
        return False

    found = False
    for queue_entry in res['queues'][queue_name]:
        if queue_entry['name'] != name:
            continue

        found = True

        if tx_hash is not None and queue_entry['tx_hash'] != tx_hash:
            res['test'] = 'tx hash mismatch: expected {}'.format(tx_hash)
            print json.dumps(res)
            error = True
            return False

        break

    if not found:
        print "name {} not found in queues".format(name)
        print json.dumps(res)
        return False

    return True
    

def scenario( wallets, **kw ):

    global wallet_keys, wallet_keys_2, error, index_file_data, resource_data

    wallet_keys = testlib.blockstack_client_initialize_wallet( "0123456789abcdef", wallets[5].privkey, wallets[3].privkey, wallets[4].privkey )
    test_proxy = testlib.TestAPIProxy()
    blockstack_client.set_default_proxy( test_proxy )

    testlib.blockstack_namespace_preorder( "test", wallets[1].addr, wallets[0].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_namespace_reveal( "test", wallets[1].addr, 52595, 250, 4, [6,5,4,3,2,1,0,0,0,0,0,0,0,0,0,0], 10, 10, wallets[0].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_namespace_ready( "test", wallets[1].privkey )
    testlib.next_block( **kw )

    testlib.blockstack_name_preorder( "foo.test", wallets[2].privkey, wallets[3].addr )
    testlib.next_block( **kw )
    
    testlib.blockstack_name_register( "foo.test", wallets[2].privkey, wallets[3].addr )
    testlib.next_block( **kw )
    
    # migrate profiles 
    res = testlib.migrate_profile( "foo.test", proxy=test_proxy, wallet_keys=wallet_keys )
    if 'error' in res:
        res['test'] = 'Failed to initialize foo.test profile'
        print json.dumps(res, indent=4, sort_keys=True)
        error = True
        return 

    # bootstrap storage for this wallet
    res = testlib.blockstack_cli_upgrade_storage("foo.test", password="0123456789abcdef")
    if 'error' in res:
        print 'failed to bootstrap storage for foo.test'
        print json.dumps(res, indent=4, sort_keys=True)
        return False

    if not blockstack_client.check_storage_setup():
        print "storage is not set up"
        return False

    # tell serialization-checker that value_hash can be ignored here
    print "BLOCKSTACK_SERIALIZATION_CHECK_IGNORE value_hash"
    sys.stdout.flush()
    
    testlib.next_block( **kw )

    config_path = os.environ.get("BLOCKSTACK_CLIENT_CONFIG", None)

    # make an index file for a dumb app 
    index_file_path = "/tmp/rest_register.index.html"
    with open(index_file_path, "w") as f:
        f.write(index_file_data)

    # register an application under foo.test
    res = testlib.blockstack_cli_app_publish("foo.test", "names,register,prices,user_read,user_write,user_admin,zonefiles,blockchain,node_read", index_file_path, appname="register", drivers="disk", password="0123456789abcdef" )
    if 'error' in res:
        res['test'] = 'Failed to register foo.test/register app'
        print json.dumps(res, indent=4, sort_keys=True)
        return False

    # make a user for bar.test (via the REST api)
    res = testlib.blockstack_cli_create_user( "foo_user_id", password="0123456789abcdef" )
    if 'error' in res:
        res['test'] = 'Failed to create user'
        print json.dumps(res)
        return False

    # make an account for bar_user_id (bar.test)
    res = testlib.blockstack_app_create_account("foo_user_id", "foo.test", "register")
    if 'error' in res:
        res['test'] = 'Failed to create account: {}'.format(res['error'])
        print json.dumps(res, indent=4, sort_keys=True)
        return False 
    
    # get session
    ses = res['ses']

    # for funsies, get the price of .test
    res = testlib.blockstack_REST_call('GET', '/api/v1/prices/namespaces/test', ses, name="foo.test", appname="register")
    if 'error' in res or res['http_status'] != 200:
        res['test'] = 'Failed to get price of .test'
        print json.dumps(res)
        return False

    test_price = res['response']['satoshis']
    print '\n\n.test costed {} satoshis\n\n'.format(test_price)

    # get the price for bar.test
    res = testlib.blockstack_REST_call('GET', '/api/v1/prices/names/bar.test', ses, name="foo.test", appname="register")
    if 'error' in res or res['http_status'] != 200:
        res['test'] = 'Failed to get price of bar.test'
        print json.dumps(res)
        return False

    bar_price = res['response']['satoshis']
    print "\n\nbar.test will cost {} satoshis\n\n".format(bar_price)
  
    # get all users (via the REST api)
    res = testlib.blockstack_REST_call('GET', '/api/v1/users', ses, name="foo.test", appname="register")
    if 'error' in res or res['http_status'] != 200:
        res['test'] = 'Failed to get all users'
        print json.dumps(res)
        return False

    # should have foo_user_id 
    all_users = res['response']
    if 'error' in all_users:
        res['test'] = 'Failed to list users'
        print json.dumps(res)
        return False

    print "\n\nall users:\n{}\n\n".format(json.dumps(all_users, indent=4, sort_keys=True))

    if len(all_users) != 1 or all_users[0]['user_id'] != 'foo_user_id':
        res['test'] = 'Got invalid users'
        print json.dumps(res)
        return False

    # query wallet 
    wallet_public = testlib.blockstack_REST_call('GET', '/api/v1/node/wallet/public', ses, name='foo.test', appname='register')
    if wallet_public['http_status'] != 200:
        wallet_public['test'] = 'failed to get wallet'
        print json.dumps(wallet_public)
        return False

    print '\n\nwallet public info:\n{}\n\n'.format(json.dumps(wallet_public, indent=4, sort_keys=True))

    # get the user (via the REST api)
    # expect 404, since the user has no profile
    res = testlib.blockstack_REST_call('GET', '/api/v1/users/foo_user_id', ses, name="foo.test", appname="register")
    if res['http_status'] != 404:
        res['test'] = 'expected http 404'
        print json.dumps(res)
        return False

    # change the user profile for foo_user_id
    res = testlib.blockstack_REST_call('PATCH', '/api/v1/users/foo_user_id', ses, name="foo.test", appname="register", data={'profile': {'hello': 'world'}})
    if 'error' in res or res['http_status'] != 200:
        res['test'] = 'Failed to set profile'
        print json.dumps(res)
        return False

    # get the profile. Should succeed
    res = testlib.blockstack_REST_call('GET', '/api/v1/users/foo_user_id', ses, name="foo.test", appname="register" )
    if 'error' in res or res['http_status'] != 200:
        res['test'] = 'Failed to get profile'
        print json.dumps(res)
        return False

    if not res['response'].has_key('hello') or res['response']['hello'] != 'world' or not res['response'].has_key('timestamp'):
        res['test'] = 'Failed to get the right profile'
        print json.dumps(res)
        return False

    # register the name bar.test 
    res = testlib.blockstack_REST_call('POST', '/api/v1/names', ses, name="foo.test", appname="register", data={'name': 'bar.test'})
    if 'error' in res:
        res['test'] = 'Failed to register user'
        print json.dumps(res)
        error = True
        return False

    print res
    tx_hash = res['response']['transaction_hash']

    # wait for preorder to get confirmed...
    for i in xrange(0, 6):
        testlib.next_block( **kw )

    res = verify_in_queue(ses, 'bar.test', 'preorder', tx_hash )
    if not res:
        return False

    # wait for the preorder to get confirmed
    for i in xrange(0, 6):
        testlib.next_block( **kw )

    in_preorder_queue = False

    # wait for the poller to pick it up
    print >> sys.stderr, "Waiting 10 seconds for the backend to submit the register"
    for i in xrange(0, 10):
        # poll
        res = testlib.blockstack_REST_call("GET", "/api/v1/names/bar.test", ses, name="foo.test", appname="register")
        if 'error' in res:
            res['test'] = 'Failed to query name'
            print json.dumps(res)
            error = True
            return False

        if res['http_status'] != 200 and res['http_status'] != 404:
            res['test'] = 'HTTP status {}, response = {}'.format(res['http_status'], res['response'])
            print json.dumps(res)
            error = True
            return False

        # should be in the preorder queue at some point 
        if res['http_status'] == 200 and res['response']['operation'] == 'preorder':
            in_preorder_queue = True

        time.sleep(1)

    # should have been preordered
    if not in_preorder_queue:
        print "Name was never preordered"
        print json.dumps(res)
        return False

    # name should be in the 'register' queue now
    if res['response']['operation'] != 'register':
        print "Name not registered"
        print json.dumps(res)
        return False

    # wait for the register to get confirmed 
    for i in xrange(0, 6):
        testlib.next_block( **kw )

    res = verify_in_queue(ses, 'bar.test', 'register', None )
    if not res:
        return False

    for i in xrange(0, 6):
        testlib.next_block( **kw )

    in_update_queue = False
    print >> sys.stderr, "Waiting 10 seconds for the backend to acknowledge registration"
    for i in xrange(0, 10):
        # poll 
        res = testlib.blockstack_REST_call("GET", "/api/v1/names/bar.test", ses, name="foo.test", appname="register")
        if 'error' in res:
            res['test'] = 'Failed to query name'
            print json.dumps(res)
            error = True
            return False

        if res['http_status'] != 200 and res['http_status'] != 404:
            res['test'] = 'HTTP status {}, response = {}'.format(res['http_status'], res['response'])
            print json.dumps(res)
            error = True
            return False

        time.sleep(1)

    # wait for update to get confirmed 
    for i in xrange(0, 6):
        testlib.next_block( **kw )

    res = verify_in_queue(ses, 'bar.test', 'update', None )
    if not res:
        return False

    for i in xrange(0, 12):
        testlib.next_block( **kw )

    print >> sys.stderr, "Waiting 10 seconds for the backend to acknowledge update"
    for i in xrange(0, 10):
        # poll 
        res = testlib.blockstack_REST_call("GET", "/api/v1/names/bar.test", ses, name="foo.test", appname="register")
        if 'error' in res:
            res['test'] = 'Failed to query name'
            print json.dumps(res)
            error = True
            return False

        if res['http_status'] != 200:
            res['test'] = 'HTTP status {}, response = {}'.format(res['http_status'], res['response'])
            print json.dumps(res)
            error = True
            return False

        time.sleep(1)

    # should now be registered 
    if res['response']['status'] != 'registered':
        print "register not complete"
        print json.dumps(res)
        return False
   
    zonefile_hash = res['response']['zonefile_hash']

    # do we have the history for the name?
    res = testlib.blockstack_REST_call("GET", "/api/v1/names/bar.test/history", ses, name="foo.test", appname="register")
    if 'error' in res or res['http_status'] != 200:
        res['test'] = "Failed to get name history for foo.test"
        print json.dumps(res)
        return False

    # valid history?
    hist = res['response']
    if len(hist.keys()) != 3:
        res['test'] = 'Failed to get update history'
        res['history'] = hist
        print json.dumps(res, indent=4, sort_keys=True)
        return False

    # get the zonefile
    res = testlib.blockstack_REST_call("GET", "/api/v1/names/bar.test/zonefile/{}".format(zonefile_hash), ses, name="foo.test", appname="register")
    if 'error' in res or res['http_status'] != 200:
        res['test'] = 'Failed to get name zonefile'
        print json.dumps(res)
        return False

def check( state_engine ):

    global wallet_keys, error, index_file_data, resource_data
    
    config_path = os.environ.get("BLOCKSTACK_CLIENT_CONFIG")
    assert config_path

    if error:
        print "Key operation failed."
        return False

    # not revealed, but ready 
    ns = state_engine.get_namespace_reveal( "test" )
    if ns is not None:
        print "namespace not ready"
        return False 

    ns = state_engine.get_namespace( "test" )
    if ns is None:
        print "no namespace"
        return False 

    if ns['namespace_id'] != 'test':
        print "wrong namespace"
        return False 

    names = ['foo.test', 'bar.test']
    wallet_keys_list = [wallet_keys, wallet_keys]
    test_proxy = testlib.TestAPIProxy()

    for i in xrange(0, len(names)):
        name = names[i]
        wallet_payer = 5
        wallet_owner = 3
        wallet_data_pubkey = 4

        # not preordered
        preorder = state_engine.get_name_preorder( name, pybitcoin.make_pay_to_address_script(wallets[wallet_payer].addr), wallets[wallet_owner].addr )
        if preorder is not None:
            print "still have preorder"
            return False
    
        # registered 
        name_rec = state_engine.get_name( name )
        if name_rec is None:
            print "name does not exist"
            return False 

        # owned 
        if name_rec['address'] != wallets[wallet_owner].addr or name_rec['sender'] != pybitcoin.make_pay_to_address_script(wallets[wallet_owner].addr):
            print "name {} has wrong owner".format(name)
            return False 

    # get app config 
    app_config = testlib.blockstack_cli_app_get_config( "foo.test", appname="register" )
    if 'error' in app_config:
        print "failed to get app config\n{}\n".format(json.dumps(app_config, indent=4, sort_keys=True))
        return False

    # inspect...
    app_config = app_config['config']

    if app_config['driver_hints'] != ['disk']:
        print "Invalid driver hints\n{}\n".format(json.dumps(app_config, indent=4, sort_keys=True))
        return False

    if 'names' not in app_config['api_methods'] or 'register' not in app_config['api_methods']:
        print "Invalid API list\n{}\n".format(json.dumps(app_config, indent=4, sort_keys=True))
        return False

    if len(app_config['index_uris']) != 1:
        print "Invalid URI records\n{}\n".format(json.dumps(app_config, indent=4, sort_keys=True))
        return False

    return True
