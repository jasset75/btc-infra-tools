#!/usr/bin/env python3
"""Read-only mining health probe; all host details come from external TOML."""
import argparse
import json
import sys
import urllib.request

from handoff import load_config, run


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
    config = load_config(parser.parse_args().config)["mining"]
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
