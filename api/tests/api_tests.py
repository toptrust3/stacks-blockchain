"""
    Blockstack Core
    ~~~~~

    copyright: (c) 2017 by Blockstack.org

This file is part of Blockstack Core.

    Blockstack Core is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    Blockstack Core is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with Search. If not, see <http://www.gnu.org/licenses/>.
"""

import os, sys, re, json, time
import unittest
import requests
import argparse
import binascii

from test import test_support
from binascii import hexlify

import api

import api.config

from api.tests.resolver_tests import ResolverTestCase
from api.tests.search_tests import SearchTestCase
from blockstack_client import schemas 

BASE_URL = 'http://localhost:5000'
API_VERSION = '1'

api.app.testing = True
app = api.app.test_client()

class FakeResponseObj:
    def __init__(self):
        self.status_code = 600
        self.data = ""

class ForwardingClient:
    def __init__(self, base_url):
        self.base_url = base_url
    def get(self, endpoint, headers = {}):
        resp = requests.get(self.base_url + endpoint, headers = headers)
        ret_obj = FakeResponseObj()
        ret_obj.status_code = resp.status_code
        ret_obj.data = resp.text
        return ret_obj

def test_get_request(cls, endpoint, headers={}, status_code=200):
    t_start = time.time()
    resp = app.get(endpoint, headers = headers)
    t_end = time.time()
    print("\r{}get time: {}s".format("\t"*9, t_end - t_start))
    if not resp.status_code == status_code:
        print(endpoint)
        print(resp.status_code)

    data = json.loads(resp.data)
    cls.assertEqual(resp.status_code, status_code)
    return data

def test_post_request(cls, endpoint, payload, headers={}, status_code=200):
    resp = app.post(endpoint, data=json.dumps(payload), headers=headers)
    data = json.loads(resp.data)
    cls.assertTrue(resp.status_code == status_code)
    return data


def check_data(cls, data, required_keys=[], banned_keys=[]):
    for k in required_keys:
        cls.assertTrue(k in data)
        for subkey in required_keys[k]:
            cls.assertTrue(subkey in data[k])
    for k in banned_keys:
        if len(banned_keys[k]) is 0:
            cls.assertTrue(k not in data)
        else:
            cls.assertTrue(k in data)
            for subkey in banned_keys[k]:
                cls.assertTrue(subkey not in data[k])


class PingTest(unittest.TestCase):
    def test_found_user_lookup(self):
        data = test_get_request(self, "/v1/ping",
                                headers = {} , status_code=200)
        
        self.assertTrue(data['status'] == 'alive')

class LookupUsersTest(unittest.TestCase):
    def build_url(self, username):
        return '/v1/names/{}'.format(username)

    def test_found_user_lookup(self):
        usernames = 'muneeb.id'
        data = test_get_request(self, self.build_url(usernames),
                                headers = {}, status_code=200)
        
        self.assertTrue(data['status'] == 'registered')
        self.assertRegexpMatches(data['address'], schemas.OP_ADDRESS_PATTERN)
        self.assertRegexpMatches(data['zonefile_hash'], schemas.OP_ZONEFILE_HASH_PATTERN)
        self.assertRegexpMatches(data['last_txid'], schemas.OP_TXID_PATTERN)

    def test_get_all_names(self):
        data = test_get_request(self, "/v1/names?page=0",
                                headers = {} , status_code=200)
        self.assertEqual(len(data), 100, "Paginated name length != 100")
        data = test_get_request(self, "/v1/names",
                                headers = {} , status_code=401)
        data = test_get_request(self, "/v1/names?page=10000",
                                headers = {} , status_code=200)

    def test_user_not_formatted(self):
        usernames = 'muneeb'
        data = test_get_request(self, self.build_url(usernames),
                                headers = {}, status_code=500)
        self.assertTrue(data['error'] == 'Failed to lookup name')


class NameHistoryTest(unittest.TestCase):
    def build_url(self, username):
        return '/v1/names/{}/history'.format(username)

    def check_history_block(self, blocks):
        self.assertEqual(len(blocks), 1)
        block = blocks[0]
        self.assertRegexpMatches(block['address'], schemas.OP_ADDRESS_PATTERN)
        self.assertTrue(block['opcode'].startswith('NAME'))

    def test_found_user_lookup(self):
        usernames = 'muneeb.id'
        data = test_get_request(self, self.build_url(usernames),
                                headers = {}, status_code=200)
        
        self.assertTrue(len(data.keys()) > 2)
        for block_key, block_data in data.items():
            self.check_history_block(block_data)


class NamesOwnedTest(unittest.TestCase):
    def build_url(self, addr):
        return '/v1/addresses/bitcoin/{}'.format(addr)
    def test_check_names(self):
        addrs_to_check = ["1QJQxDas5JhdiXhEbNS14iNjr8auFT96GP",
                          "16EMaNw3pkn3v6f2BgnSSs53zAKH4Q8YJg"]
        names_to_check = ["muneeb.id", "judecn.id"]
        for addr, name in zip(addrs_to_check, names_to_check):
            data = test_get_request(self, self.build_url(addr),
                                    headers = {}, status_code = 200)
            self.assertTrue(len(data["names"]) > 0)
            self.assertIn(name, data["names"])

class NamespaceTest(unittest.TestCase):
    def test_id_space(self):
        data = test_get_request(self, "/v1/namespaces",
                                headers = {} , status_code=200)        
        self.assertIn('id', data)
    def test_id_space_names(self):
        data = test_get_request(self, "/v1/namespaces/id/names?page=0",
                                headers = {} , status_code=200)
        self.assertEqual(len(data), 100, "Paginated name length != 100")
        data = test_get_request(self, "/v1/namespaces/id/names",
                                headers = {} , status_code=401)
        


class NamepriceTest(unittest.TestCase):
    def price_url(self, name):
        return "/v1/prices/names/{}".format(name)
    
    def test_id_price(self):
        data = test_get_request(self, self.price_url("muneeb.id"),
                                headers = {} , status_code=200)        
        json_keys = data.keys()
        self.assertIn('name_price', json_keys)
        self.assertIn('preorder_tx_fee', json_keys)
        self.assertIn('register_tx_fee', json_keys)
        self.assertIn('total_estimated_cost', json_keys)
        self.assertIn('total_tx_fees', json_keys)
        self.assertIn('update_tx_fee', json_keys)


class SearchAPITest(unittest.TestCase):
    def search_url(self, q):
        return "/v1/search?query={}".format(q)

    def test_forward_to_search_server(self):
        u = "muneeb"
        original = api.config.SEARCH_API_ENDPOINT_ENABLED
        api.config.SEARCH_API_ENDPOINT_ENABLED = False
        
        data = test_get_request(self, self.search_url(u),
                                headers = {}, status_code=200)

        self.assertTrue(len(data['results']) > 0)
        self.assertIn(u, data['results'][0]['username'])
        self.assertIn("profile", data['results'][0].keys())

        api.config.SEARCH_API_ENDPOINT_ENABLED = original
        
    def test_search_server(self):
        u = "muneeb"
        if not api.config.SEARCH_API_ENDPOINT_ENABLED:
            print("skipping search server test")
            return
        data = test_get_request(self, self.search_url(u),
                                headers = {}, status_code=200)

        self.assertTrue(len(data['results']) > 0)
        self.assertIn(u, data['results'][0]['username'])
        self.assertIn("profile", data['results'][0].keys())

class TestAPILandingPageExamples(unittest.TestCase):
    def test_endpoints(self):
        from api.utils import get_api_calls
        current_dir = os.path.abspath(os.path.dirname(__file__))
        api_endpoints = [ call['tryit_pathname'] 
                          for call in get_api_calls(current_dir + '/../api_v1.md')
                          if (not ("private" in call and call["private"].lower().startswith("t")))
                          and 'tryit_pathname' in call ]
        print("")
        for url in api_endpoints:
            print("\tTesting API example: {}".format(url))
            test_get_request(self, url, headers = {}, status_code=200)

class ConsensusTest(unittest.TestCase):
    def test_id_space(self):
        data = test_get_request(self, "/v1/blockchains/bitcoin/consensus",
                                headers = {} , status_code=200)        
        self.assertRegexpMatches(data['consensus_hash'], schemas.OP_CONSENSUS_HASH_PATTERN)


def test_main(args = []):
    test_classes = [PingTest, LookupUsersTest, NamespaceTest, ConsensusTest, TestAPILandingPageExamples,
                    NamepriceTest, NamesOwnedTest, NameHistoryTest, SearchAPITest]
    test_classes += [ResolverTestCase]
    if api.config.SEARCH_API_ENDPOINT_ENABLED:
        test_classes += [SearchTestCase]

    test_map = {}
    for t in test_classes:
        test_map[t.__name__] = t


    with test_support.captured_stdout() as out:
        try:
            test_support.run_unittest(PingTest)
        except Exception as e:
            print(e)
    out = out.getvalue()
    if out[-3:-1] != "OK":
        print(out)
        print("Failure of the ping test means the rest of the unit tests will fail. Is the blockstack api daemon running? (did you run `blockstack api start`)")
        return

    if len(args) == 1 and args[0] == "--list":
        print("Tests supported: ")
        for testname in test_map.keys():
            print(testname)
        return

    if "--remote" in args:
        ainx = args.index("--remote")
        del args[ainx]
        global app
        app = ForwardingClient(args[ainx])
        del args[ainx]

    if len(args) == 0 or args[0] == "--all":
        args = [ testname for testname in test_map.keys() ]

    test_support.run_unittest( *[test_map[test_name] for test_name in args] )

if __name__ == '__main__':
    test_main(sys.argv[1:])
