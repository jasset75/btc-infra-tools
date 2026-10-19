#!/usr/bin/env python3
"""Read-only mining health probe; all host details come from external TOML."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib
import urllib.request


def expand(value):
    if isinstance(value, str):
        return os.path.expandvars(os.path.expanduser(value))
    if isinstance(value, list):
        return [expand(x) for x in value]
    if isinstance(value, dict):
        return {k: expand(v) for k, v in value.items()}
    return value


def run(argv, timeout=5):
    return subprocess.run(argv, capture_output=True, text=True, check=True, timeout=timeout)


def healthy(blockchain, network, miner, tcp, config):
    return (blockchain["blocks"] == blockchain["headers"]
            and blockchain["initialblockdownload"] is False
            and network["networkactive"] is True
            and network["connections"] > 0
            and miner["isUsingFallbackStratum"] == 0
            and miner["stratumURL"] == config["stratum_host"]
            and miner["stratumPort"] == config["stratum_port"]
            and miner["hashRate"] > 0
            and config["tcp_peer"] in tcp)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", required=True)
    config = expand(tomllib.loads(Path(parser.parse_args().config).read_text()))["mining"]
    try:
        blockchain = json.loads(run(config["bitcoin_cli"] + ["getblockchaininfo"], timeout=5).stdout)
        network = json.loads(run(config["bitcoin_cli"] + ["getnetworkinfo"], timeout=5).stdout)
        with urllib.request.urlopen(config["miner_url"], timeout=5) as response:
            miner = json.load(response)
        tcp = run(["lsof", "-nP", "-iTCP:%s" % config["stratum_port"], "-sTCP:ESTABLISHED"], timeout=5).stdout
        ok = healthy(blockchain, network, miner, tcp, config)
    except Exception:
        # Unknown health inhibits optional work. Do not print CLI/API details.
        ok = False
    print(json.dumps({"mining_healthy": bool(ok)}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
