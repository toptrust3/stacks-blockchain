# Blockstore

[![PyPI](https://img.shields.io/pypi/v/blockstore.svg)](https://pypi.python.org/pypi/blockstore/)
[![PyPI](https://img.shields.io/pypi/dm/blockstore.svg)](https://pypi.python.org/pypi/blockstore/)
[![Slack](http://slack.blockstack.org/badge.svg)](http://slack.blockstack.org/)

### Name Registrations on the Bitcoin Blockchain

Blockstore enables human-readable name registrations on the Bitcoin blockchain, along with the ability to store associated data in external datastores. You can use it to register globally unique names, associate data with those names, and transfer them between Bitcoin addresses. Anyone can perform lookups on those names and securely obtain the data associated with them.

Blockstore uses the Bitcoin blockchain for storing name operations and data hashes, and the Kademlia-based distributed hash table (DHT) and other external datastores for storing the full data files outside of the blockchain.

## Installation

**NOTE: This repo is going through rapid development. If you notice any issues during installation etc please report them in Github issues. We hope to have a stable, easy to install, release out very soon!**

The fastest way to get started with blockstore is to use pip:

```
pip install blockstore
```

### What's included

Within the install you'll find the following directories and files. You'll see something like this:

```
blockstore/
├── bin/
│   ├── blockstored
│   └── README.md
├── blockstore/
│   ├── __init__.py
│   ├── blockmirrord.py
│   ├── blockmirrord.tac
│   ├── blockstore.tac
│   ├── blockstored.py
│   ├── build_nameset.py
│   ├── coinkit.patch
│   ├── dht/
│   │   ├── __init__.py
│   │   ├── image/
│   │   │   ├── Dockerfile
│   │   │   └── README.md
│   │   ├── plugin.py
│   │   ├── README.md
│   │   ├── server.tac
│   │   ├── storage.py
│   │   └── test.py
│   ├── lib/
│   │   ├── __init__.py
│   │   ├── b40.py
│   │   ├── config.py
│   │   ├── hashing.py
│   │   ├── nameset/
│   │   │   ├── __init__.py
│   │   │   ├── namedb.py
│   │   │   └── virtualchain_hooks.py
│   │   ├── operations/
│   │   │   ├── __init__.py
│   │   │   ├── nameimport.py
│   │   │   ├── namespacepreorder.py
│   │   │   ├── namespaceready.py
│   │   │   ├── namespacereveal.py
│   │   │   ├── preorder.py
│   │   │   ├── register.py
│   │   │   ├── revoke.py
│   │   │   ├── transfer.py
│   │   │   └── update.py
│   │   ├── README.md
│   │   └── scripts.py
│   ├── tests/
│   │   └── unit_tests.py
│   └── TODO.txt
├── Dockerfile
├── images/
│   └── Dockerfile
├── LICENSE
├── MANIFEST.in
├── README.md
├── requirements.txt
└── setup.py
```


## Getting Started

Start blockstored and index the blockchain:

```
$ blockstored start
```

Then, perform name lookups:

```
$ blockstore-cli lookup swiftonsecurity
{
    "data": "{\"name\":{\"formatted\": \"Taylor Swift\"}}"
}
```

Next, learn how to register names of your own, as well as transfer them and associate data with them:

[Full usage docs](../../wiki/Usage)

## Design

[Design decisions](../../wiki/Design-Decisions)

[Protocol details](../../wiki/Protocol-Details)

[Definitions](../../wiki/Definitions)

[FAQ](../../wiki/FAQ)

## Contributions

The best way to contribute is to:

1. decide what changes you'd like to make (you can find inspiration in the tab of issues)
1. fork the repo
1. make your changes
1. submit a pull request

[Code contributors](../../graphs/contributors)

[Full contributor list](../../wiki/Contributors)

## License

GPL v3. See LICENSE.

Copyright: (c) 2015 by Blockstack.org
