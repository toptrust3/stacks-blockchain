# Blockstack Core

[![PyPI](https://img.shields.io/pypi/v/blockstack.svg)](https://pypi.python.org/pypi/blockstack-server/)
[![Slack](http://slack.blockstack.org/badge.svg)](http://slack.blockstack.org/)

Blockstack is a new decentralized web, where users own their data and apps run on user devices without the need for hosting. Developers can build
serverless apps without the need to trust any centralized services or infrastructure.

For more info on Blockstack see: http://github.com/blockstack/blockstack

**Blockstack Core is the reference implementation of Blockstack.** It is responsible for processing blockchain transactions, creating virtualchain state, and building the peer network amongst other things. Blockstack Core provides a RESTful interface for clients and also comes with a command-line-interface (CLI).

## Table of Contents

- [Quick Start](#quick-start)
- [Development Status](#development-status)
- [Blockstack Docs](#blockstack-docs)
- [Contributing](#contributing)
- [Community](#community)

## Quick Start

The fastest way to get started with Blockstack is with pip.

For Debian & Ubuntu:
```
$ sudo apt-get update && sudo apt-get install -y python-pip python-dev libssl-dev libffi-dev rng-tools libgmp3-dev
$ sudo pip install blockstack
```
For macOS: 
```
$ brew install gmp libffi openssl
$ sudo pip install --upgrade pip
$ sudo pip install blockstack
```

You can test your installation by trying:
```
$ blockstack info 
```
which should display the last block processed and the latest consensus hash.

After installation, you can start Blockstack Core and index the blockchain:
```bash
$ blockstack-core --debug start
$ tail -f ~/.blockstack-server/blockstack-server.logexit
```

Next, visit the [basic usage docs](https://blockstack.org/docs/basic-usage) and [extended usage docs](https://blockstack.org/docs/basic-usage) to learn how to register names of your own, as well as transfer them and associate data with them.

If you encounter any technical issues in installing or using Blockstack, please [search the open issues](https://github.com/blockstack/blockstack-core/issues) and start a new one if your issue is not covered. 

## Development Status

**v0.14.1** is the current stable release of Blockstack Core (available on the master branch).<br>
**v0.14.2** is the next release candidate for Blockstack Core (available on the [v0.14.2 branch](https://github.com/blockstack/blockstack-core/tree/rc-0.14.2)). See the [release notes](https://github.com/blockstack/blockstack-core/blob/master/release_notes/changelog-0.14.1.md).

Most of the development is happening in the [v0.14.2 branch](https://github.com/blockstack/blockstack-core/tree/rc-0.14.2). Please submit all
pull requests to that branch.

## Blockstack Docs

You can learn more by visiting [the Blockstack Website](https://blockstack.org) and checking out the in-depth articles and documentation:

- [How Blockstack Works](https://blockstack.org/docs/how-blockstack-works)
- [Blockstack vs. DNS](https://blockstack.org/docs/blockstack-vs-dns)
- [Blockstack vs. Namecoin](https://blockstack.org/docs/blockstack-vs-namecoin)
- [Blockstack Namespaces](https://blockstack.org/docs/namespaces)
- [Blockstack Light Clients](https://blockstack.org/docs/light-clients)

You can also read the Blockstack paper:

- ["Blockstack: A Global Naming and Storage System Secured by Blockchains"](https://blockstack.org/blockstack.pdf), Proc. USENIX Annual Technical Conference (ATC ’16), June 2016

If you have high-level questions about Blockstack, try [searching our forum](https://forum.blockstack.org) and start a new question if your question is not answered there.

## Contributing

We welcome any small or big contributions! Please take a moment to
[review the guidelines for contributing to open source](https://guides.github.com/activities/contributing-to-open-source/) in order to make the contribution process easy and effective for everyone involved.

**Developers**:  You can try out Blockstack Core in a local sandbox using our [integration test framework](https://github.com/blockstack/blockstack-integration-tests).

You can install the latest release candidate by:
```bash
$ git clone https://github.com/blockstack/blockstack-core.git
$ blockstack-core/images/scripts/debian-release-candidate.sh
```

## Community

We have an active community of developers and the best place to interact with the community is:

- [Mailing List](http://blockstack.us14.list-manage1.com/subscribe?u=394a2b5cfee9c4b0f7525b009&id=0e5478ae86) (3,000+ members)
- [Blockstack Forum](http://forum.blockstack.org)
- [Live chat on Slack](http://chat.blockstack.org/) (2,400+ members)

## Copyright and License

The code and documentation copyright are attributed to blockstack.org for the year of 2017.

This code is released under
[the GPL v3 license](http://www.gnu.org/licenses/quick-guide-gplv3.en.html), and the docs are released under [the Creative Commons license](http://creativecommons.org/).

## Command-Line Reference

### Listing All Commands

```bash
$ blockstack
usage: blockstack [-h]
                  ...

Blockstack cli version 0.14.0
positional arguments:
    balance             Get the account balance
    configure           Interactively configure the client
    deposit             Display the address with which to receive bitcoins
    import              Display the address with which to receive names
    info                Get details about pending name commands
    lookup              Get the zone file and profile for a particular name
    migrate             Migrate a profile to the latest profile format
    names               Display the names owned by local addresses
    ping                Check server status and get server details
    price               Get the price of a name
    register            Register a name
    renew               Renew a name
    revoke              Revoke a name
    set_advanced_mode   Enable advanced commands
    transfer            Transfer a name to a new address
    update              Set the zone file for a name
    whois               Look up the blockchain info for a name

optional arguments:
  -h, --help            show this help message and exit
```

### Info (or ping or status)

```bash
$ blockstack info
```

##### Examples

```bash
$ blockstack info
{
    "advanced_mode": false,
    "cli_version": "0.14.0",
    "consensus_hash": "106d4648661d49e16d103b071e26617e",
    "last_block_processed": 420518,
    "last_block_seen": 420596,
    "server_alive": true,
    "server_host": "40.76.8.249",
    "server_port": "6264",
    "server_version": "0.14.0"
}
```

### Config

```bash
$ blockstack configure
```

##### Examples

```bash
$ blockstack  configure
---------------------------------------------------------
Your client does not have enough information to connect
to a Blockstack server.  Please supply the following
parameters, or press [ENTER] to select the default value.
---------------------------------------------------------
blockchain_headers (default: '/home/jude/.blockstack/blockchain-headers.dat'):
blockchain_writer (default: 'blockcypher'):
api_endpoint_port (default: '6270'):
poll_interval (default: '300'):
metadata (default: '/home/jude/.blockstack/metadata'):
server (default: 'node.blockstack.org'):
advanced_mode (default: 'False'):
blockchain_reader (default: 'blockcypher'):
email (default: ''):
rpc_token (default: '2dbf700c6c0d546be23ad7ae4e5e1bbb6cdaa10a3ae4deca8e598bf9ec58fc6a'):
storage_drivers_required_write (default: 'disk,blockstack_server'):
queue_path (default: '/home/jude/.blockstack/queues.db'):
storage_drivers (default: 'disk,blockstack_resolver,blockstack_server,http,dht'):
path (default: '/home/jude/.blockstack/client.ini'):
client_version (default: '0.0.13.4'):
rpc_detach (default: 'True'):
port (default: '6264'):
dir (default: '/home/jude/.blockstack/client.ini'):
anonymous_statistics (default: 'True'):
--------------------------------------------------------
Blockstack does not have enough information to connect
to bitcoind.  Please supply the following parameters, or
press [ENTER] to select the default value.
--------------------------------------------------------
mock (default: 'False'):
passwd (default: 'blockstacksystem'):
server (default: 'bitcoin.blockstack.com'):
user (default: 'blockstack'):
timeout (default: '300.0'):
port (default: '8332'):
use_https (default: 'False'):
-------------------------------
Blockchain reader configuration
----------------------------------------
Please enter your Blockcypher API token.
----------------------------------------
api_token (default: ''):
-------------------------------
Blockchain writer configuration
----------------------------------------
Please enter your Blockcypher API token.
----------------------------------------
api_token (default: ''):
Saving configuration to /home/jude/.blockstack/client.ini
{
    "path": "/home/jude/.blockstack/client.ini"
}
```

### Cost

```bash
$ blockstack price <name>
```

##### Examples

```bash
$ blockstack price $(whoami).id
{
    "name_price": {
        "btc": "0.00025",
        "satoshis": "25000"
    },
    "preorder_tx_fee": {
        "btc": "0.00047406",
        "satoshis": "47406"
    },
    "register_tx_fee": {
        "btc": "0.00046184",
        "satoshis": "46184"
    },
    "total_estimated_cost": {
        "btc": "0.00188394",
        "satoshis": "188394"
    },
    "update_tx_fee": {
        "btc": "0.00069804",
        "satoshis": "69804"
    }
}
```

### Whois

```bash
$ blockstack whois <name>
```

##### Examples

```bash
$ blockstack whois fredwilson.id
{
    "block_preordered_at": 374084,
    "block_renewed_at": 374084,
    "expire_block": 426679,
    "has_zonefile": true,
    "last_transaction_id": "2986ec31ec957692d7f5bc58a3b02d2ac2d1a60039e9163365fc954ff51aeb5a",
    "owner_address": "1F2nHEDLRJ39XxAvSxwQhJsaVzvS5RHDRM",
    "owner_script": "76a91499e7f97f5d2c77b4f32b4ed9ae0f0385c45aa5c788ac",
    "zonefile_hash": "1a587366368aaf8477d5ddcea2557dcbcc67073e"
}
```

```bash
$ blockstack whois $(whoami)_$(date +"%m_%d").id
Not found.
```

### Lookup

```bash
$ blockstack lookup <name>
```

##### Examples

```bash
$ blockstack lookup fredwilson.id
{
    "profile": {
        "avatar": {
            "url": "https://s3.amazonaws.com/kd4/fredwilson1"
        },
        "bio": "I am a VC",
   ...
}

```

```bash
$ blockstack lookup $(whoami)_$(date +"%m_%d").id
Not found.
```

### Register

```bash
$ blockstack register <name>
```

##### Example

```bash
$ blockstack register $(whoami)_$(date +"%m_%d").id
Registering muneeb_02_22.id will cost 0.0002225 BTC. Continue? (y/n): y
{
    "transaction_hash": "f576313b2ff4cc7cb0d25545e1e38e2d0d48a6ef486b7118e5ca0f8e8b98ae45",
    "message": "The name has been queued up for registration and will take a few hours to go through. You can check on the status at any time by running 'blockstack info'."
    "success": true
}
```

```bash
$ blockstack register fredwilson.id
fredwilson.id is already registered.
```

### Update

```bash
$ blockstack update <name> <data string or file with data>
```

##### Examples

```bash
$ echo > new_zone_file.txt <<EOF
$ORIGIN swiftonsecurity.id
$TTL 3600
pubkey TXT "pubkey:data:04cabba0b5b9a871dbaa11c044066e281c5feb57243c7d2a452f06a0d708613a46ced59f9f806e601b3353931d1e4a98d7040127f31016311050bedc0d4f1f62ff"
_file IN URI 10 1 "file:///Users/TaylorSwift/.blockstack/storage-disk/mutable/swiftonsecurity.id"
_https._tcp IN URI 10 1 "https://blockstack.s3.amazonaws.com/swiftonsecurity.id"
_http._tcp IN URI 10 1 "http://node.blockstack.org:6264/RPC2#swiftonsecurity.id"
_dht._udp IN URI 10 1 "dht+udp://fc4d9c1481a6349fe99f0e3dd7261d67b23dadc5"
EOF

$ blockstack update swiftonsecurity.id new_zone_file.txt
{
    "success": true,
    "transaction_hash": "4e1f292c09ad8e03a5f228b589d9a7dc3699b495862bee3b40f2432ac497b134",
    "message": "The name has been queued up for update and will take ~1 hour to process. You can check on the status at any time by running 'blockstack info'."
}
```

```bash
$ blockstack update muneeb.id '{"$origin": "muneeb.id", "$ttl": "3600", "uri": [{"name": "@", "priority": "10", "weight": "1", "target": "https://muneeb.ali/muneeb.id"}]}'
{
    "success": true,
    "transaction_hash": "4e1f292c09ad8e03a5f228b589d9a7dc3699b495862bee3b40f2432ac497b134",
    "message": "The name has been queued up for update and will take ~1 hour to process. You can check on the status at any time by running 'blockstack info'."
}
```

```bash
$ blockstack update fredwilson.id '{}'
Invalid $origin; must use your name

$ blockstack update fredwilson.id '{"$origin": "fredwilson.id"}'
Missing $ttl; please supply a positive integer

$ blockstack update fredwilson.id '{"$origin": "fredwilson.id", "$ttl": "3600"}'
Zonefile is missing or has invalid URI and/or TXT records

$ blockstack update fredwilson.id '{"$origin": "fredwilson.id", "$ttl": "3600", "uri": [{"name": "@", "priority": "10", "weight": "1", "target": "https://blockstack.s3.amazonaws.com/fredwilson.id"}]}'
fredwilson.id is not in your possession.
```

### Transfer

```bash
$ blockstack transfer <name> <address>
```

##### Examples

```bash
$ blockstack transfer $(whoami)_$(date +"%m_%d").id 1Jbcrh9Lkwm73jXyxramFukViEtktwq8gt
{
    "transaction_hash": "8a68d52d70cf06d819eb72a9a58f4dceda942db792ceb35dd333f43f55fa8713",
    "message": "The name has been queued up for transfer and will take ~1 hour to process. You can check on the status at any time by running 'blockstack info'."
    "success": true
}
```

```bash
$ blockstack transfer fredwilson.id 1Jbcrh9Lkwm73jXyxramFukViEtktwq8gt
fredwilson.id is not in your possession.
```

### Balance

```bash
$ blockstack balance
```

##### Examples

```bash
$ blockstack balance
{
    "addresses": [
        {
            "address": "16yE3e928JakaXbympwSywyrJPM9cuL4wZ",
            "bitcoin": 0.00959454,
            "satoshis": 959454
        }
    ],
    "total_balance": {
        "bitcoin": 0.00959454,
        "satoshis": 959454
    }
}
```

### Names

```bash
$ blockstack names
```

##### Examples

```bash
$ blockstack names
{
    "addresses": [
        {
            "address": "16CtpS8LhmW3bGtVC69UGZ3wSwvi95BE8E",
            "names_owned": [
                "testregistration001.id",
                "testregistration002.id"
            ]
        }
    ],
    "names_owned": [
        "testregistration001.id",
        "testregistration002.id"
    ]
}
```

### Deposit

```bash
$ blockstack deposit
```

##### Examples

```bash
$ blockstack deposit
{
    "address": "1EHgqHVpA1tjn6RhaVj8bx6y5NGvBwoMNS",
    "message": "Send bitcoins to the address specified.",
}
```

### Import

```bash
$ blockstack import
```

##### Examples

```bash
$ blockstack import
{
    "address": "1Jbcrh9Lkwm73jXyxramFukViEtktwq8gt"
    "message": "Send the name you want to receive to the address specified.",
}
```

## Troubleshooting Installation

**a) Error installing pycrypto**

If you see the following error, while pycrpyto installs on OS X:

```bash
error: command 'cc' failed with exit status 1
```

Try installing it with the following:

```bash
$ ARCHFLAGS=-Wno-error=unused-command-line-argument-hard-error-in-future pip install pycrypto
```

**b) Blockstack hangs while running in a VM**

If Blockstack hangs while performing one of the above operations while running in a VM, and you hit Ctrl+C, you
may see a stack trace like this:

```
Traceback (most recent call last):
  File "/home/dev/blockstack-venv/bin/blockstack", line 67, in <module>
    result = run_cli()
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/blockstack_client/cli.py", line 287, in run_cli
    result = method( args, config_path=config_path )
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/blockstack_client/actions.py", line 479, in cli_price
    fees = get_total_registration_fees( fqu, payment_privkey_info, owner_privkey_info, proxy=proxy, config_path=config_path, payment_address=payment_address )
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/blockstack_client/actions.py", line 271, in get_total_registration_fees
    preorder_tx_fee = estimate_preorder_tx_fee( name, data['satoshis'], payment_address, utxo_client, owner_privkey_params=get_privkey_info_params(owner_privkey_info), config_path=config_path, include_dust=True )
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/blockstack_client/backend/nameops.py", line 116, in estimate_preorder_tx_fee
    fake_privkey = make_fake_privkey_info( owner_privkey_params )
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/blockstack_client/backend/nameops.py", line 103, in make_fake_privkey_info
    return virtualchain.make_multisig_wallet( m, n )
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/virtualchain/lib/blockchain/bitcoin_blockchain/multisig.py", line 82, in make_multisig_wallet
    pk = BitcoinPrivateKey().to_wif()
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/pybitcoin/privatekey.py", line 55, in __init__
    secret_exponent = random_secret_exponent(self._curve.order)
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/pybitcoin/privatekey.py", line 32, in random_secret_exponent
    random_hex = hexlify(dev_random_entropy(32))
  File "/home/dev/blockstack-venv/local/lib/python2.7/site-packages/utilitybelt/entropy.py", line 38, in dev_random_entropy
    return open("/dev/random", "rb").read(numbytes)
KeyboardInterrupt
```

If so, the reason is because the VM does not have enough entropy.  This causes reads to `/dev/random` to block
for a long time.

The solution is to install `rng-tools` and configure it to seed `/dev/random` with entropy from `/dev/urandom`.
Please see your distribution documentation for setting up `rng-tools`.

If the issue you are experiencing is not listed here, please
[report it as a new issue](https://github.com/blockstack/blockstack-client/issues/new).

## Running Your Server

The CLI by default talks to a remote server, but you can easily start your own server.

Open a new terminal window and run the following command:

```bash
$ blockstack-server start --foreground
```

You can now switch the cli to use the local server:

```bash
$ blockstack configure
...
server (default: 'node.blockstack.org'): 127.0.0.1
...
```

[More information on the Blockstack Server(http://github.com/blockstack/blockstack-server)

## Client Library

You can also import the blockstack client and write your own programs.

Here is some example code to get you started:

```python
from blockstack_client import client
from blockstack_client.utils import print_result as pprint

client.session(server_host='127.0.0.1', server_port=6264)
resp = client.ping()
pprint(resp)
```

