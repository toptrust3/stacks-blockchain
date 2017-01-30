#!/usr/bin/env python
# -*- coding: utf-8 -*-

from __future__ import print_function

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

"""
IMPROTANT: READ THIS FIRST

Do NOT add CLI commands to this file.
Instead, define the appropriate method in the `actions.py` file
in this module.

This module will load and register each appropriate method from `actions.py`
as a command-line option.
"""

import argparse
import sys

import requests
requests.packages.urllib3.disable_warnings()

import logging
logging.disable(logging.CRITICAL)

from blockstack_client import config
from blockstack_client.client import session, check_storage_setup 
from blockstack_client.config import CONFIG_PATH, VERSION, semver_match
from blockstack_client.method_parser import parse_methods, build_method_subparsers

from wallet import *
from utils import exit_with_error, print_result

log = config.get_logger()


def get_methods(prefix, module):
    """
    Get the built-in CLI methods
    """
    methods = []
    for attr in dir(module):
        if not attr.startswith(prefix):
            continue

        method = getattr(module, attr)

        if callable(method) or hasattr(method, '__call__'):
            methods.append(method)

    return methods


def get_plugin_methods(module_name, prefix):
    """
    Load methods from a given module
    Return the list on success
    Return None on error
    """
    try:
        module = __import__(module_name)
    except ImportError:
        log.error('Failed to import "{}"'.format(module_name))
        return None

    return get_methods(prefix, module)


def get_cli_methods():
    """
    Get built-in CLI methods
    """
    import blockstack_client.actions as builtin_methods
    all_methods = get_methods('cli_', builtin_methods)
    return all_methods


def prompt_args(arginfolist, prompt_func):
    """
    Prompt for args, using parsed method information
    Use prompt_func(help, name) to do the prompt
    Return a list of parsed arguments
    Return None on error
    """
    parsed_args = []
    for arg in arginfolist:
        name, help = arg['name'], arg['help']

        try:
            parsed_arg = None
            while True:
                try:
                    parsed_arg = prompt_func(help, name)
                    break
                except ValueError:
                    print('Invalid args. Please try again. {}:{}'.format(name, help))
                    continue

            parsed_args.append(parsed_arg)
        except KeyboardInterrupt:
            print('Keyboard interrupt')
            return None
        except Exception as e:
            log.exception(e)
            return None

    return parsed_args


def find_arg(argv, has_arg, short_opt, long_opt):
    """
    Find an option in an argument vector.
    If @has_arg is True, then the argument will be removed as well.
    Otherwise, the argument is assumed to be True

    Return (new argv, argument) on success.  The option and its argument will be removed.
    Return (None, None) if the option is present, but no argument is given and has_arg is True.
    If the option is not found, then argv will be unchanged, and None will be returned
    """
    arg = False
    if short_opt in argv or long_opt in argv:
        i = 1
        while i < len(argv):
            if argv[i] == short_opt or argv[i] == long_opt:
                if has_arg:
                    if i + 1 >= len(argv) or argv[i+1].startswith('-'):
                        print('{}: missing argument'.format(argv[i], file=sys.stderr))
                        return (None, None)

                    arg = argv[i + 1]
                    argv.pop(i)
                    argv.pop(i)

                else:
                    argv.pop(i)
                    arg = True

            else:
                i += 1

    return (argv, arg)


def run_cli(argv=None, config_path=CONFIG_PATH):
    """
    Run a CLI command from arguments (defaults to sys.argv)
    Return the result of the command on success.
    The result will be a dict, and will have 'error' defined on error condition.
    """

    if argv is None:
        argv = sys.argv

    cli_argv = False
    cli_default_yes = False

    if '-v' in argv or '--version' in argv:
        print(VERSION)
        sys.exit(0)

    # alternative config path?
    new_argv, cli_config_path = find_arg(argv, True, '-c', '--config')
    if new_argv is None:
        # invalid
        sys.exit(1)
    
    argv = new_argv
    if cli_config_path:
        cli_argv = True
        config_path = cli_config_path
        log.debug('Use config file {}'.format(config_path))

    # CLI-given password?
    new_argv, password = find_arg(argv, True, '-p', '--password')
    if new_argv is None:
        # invalid
        sys.exit(1)

    argv = new_argv
    if password:
        log.debug("Use CLI password")
        os.environ["BLOCKSTACK_CLIENT_WALLET_PASSWORD"] = password

    # assume YES to all prompts?
    new_argv, cli_default_yes = find_arg(argv, False, '-y', '--yes')
    if new_argv is None:
        # invalid
        sys.exit(1)

    if cli_default_yes:
        log.debug("Assume YES to all interactive prompts")
        os.setenv("BLOCKSTACK_CLIENT_INTERACTIVE_YES", "1")

    conf = config.get_config(path=config_path)
    if conf is None:
        return {'error': 'Failed to load config'}

    conf_version = conf.get('client_version', '')
    if not semver_match(conf_version, VERSION):
        # back up the config file 
        if not cli_argv:
            # default config file
            backup_path = config.backup_config_file(config_path=config_path)
            if not backup_path:
                exit_with_error("Failed to back up legacy configuration file {}".format(config_path))

            else:
                exit_with_error("Backed up legacy configuration file from {} to {} and re-generated a new, default configuration.  Please restart.".format(config_path, backup_path))


    advanced_mode = conf.get('advanced_mode', False)

    parser = argparse.ArgumentParser(
        description='Blockstack cli version {}'.format(config.VERSION)
    )

    all_methods = []
    subparsers = parser.add_subparsers(dest='action')

    # add basic methods
    all_method_names = get_cli_methods()
    all_methods = parse_methods(all_method_names)
    build_method_subparsers(subparsers, all_methods)

    if not advanced_mode:
        # remove advanced methods 
        all_methods = filter( lambda m: 'advanced' not in m['pragmas'], all_methods ) 

    # Print default help message, if no argument is given
    if len(argv) == 1 or '-h' in argv or '--help' in argv:
        parser.print_help()
        return {}

    interactive, args, directive = False, None, None

    try:
        args, unknown_args = parser.parse_known_args(args=argv[1:])
        directive = args.action
    except SystemExit:
        # bad arguments
        # special case: if the method is specified, but no method arguments are given,
        # then switch to prompting the user for individual arguments.
        try:
            directive_parser = argparse.ArgumentParser(
                description='Blockstack cli version {}'.format(config.VERSION)
            )
            directive_subparsers = directive_parser.add_subparsers(
                dest='action'
            )

            # only parse the directive
            build_method_subparsers(
                directive_subparsers, all_methods, include_args=False, include_opts=False
            )
            directive_args, directive_unknown_args = directive_parser.parse_known_args(
                args=argv[1:]
            )

            # want interactive prompting
            interactive, directive = True, directive_args.action

        except SystemExit:
            # still invalid
            parser.print_help()
            return {'error': 'Invalid arguments.  Try passing "-h".'}

    result = {}

    blockstack_server, blockstack_port = conf['server'], conf['port']

    # initialize blockstack connection
    session(
        conf=conf, server_host=blockstack_server,
        server_port=blockstack_port, set_global=True
    )

    prompt_func = lambda help, name: raw_input('optional: {} ("{}"): '.format(help, name))

    # dispatch to the apporpriate method
    for method_info in all_methods:
        if directive != method_info['command']:
            continue

        method = method_info['method']
        pragmas = method_info['pragmas']

        if 'check_storage' in pragmas:
            # verify that we have set up storage
            res = check_storage_setup(config_path=config_path)
            if 'error' in res:
                return {'error': 'Please run the `upgrade_storage` command first'}

        # interactive?
        if interactive:
            print('')
            print('Interactive prompt engaged.  Press Ctrl+C to quit')
            print('Help for "{}": {}'.format(method_info['command'], method_info['help']))
            print('')

            required_args = prompt_args(method_info['args'], prompt_func)
            if required_args is None:
                return {'error': 'Failed to prompt for arguments'}

            optional_args = prompt_args(method_info['opts'], prompt_func)
            if optional_args is None:
                return {'error': 'Failed to prompt for arguments'}

            full_args = [method_info['command']] + required_args + optional_args
            try:
                args, unknown_args = parser.parse_known_args(args=full_args)
            except SystemExit:
                # invalid arguments
                return {'error': 'Invalid arguments.  Please try again.'}

        result = method(args, config_path=config_path)
        return result

    # not found
    return {'error': 'No such command "{}"'.format(args.action)}


if __name__ == '__main__':
    result = run_cli()
    if 'error' in result:
        exit_with_error(result['error'])
    else:
        print_result(result)
        sys.exit(0)
