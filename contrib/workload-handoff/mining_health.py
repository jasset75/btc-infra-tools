#!/usr/bin/env python3
"""Read-only mining health probe; all host details come from external TOML."""
import argparse
import base64
import ipaddress
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tomllib
import urllib.parse
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


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def rpc_query(config, method):
    if method not in ("getblockchaininfo", "getnetworkinfo"):
        raise ValueError("unsupported health RPC method")
    endpoint = urllib.parse.urlsplit(config["endpoint"])
    host = endpoint.hostname
    loopback = host == "localhost"
    if not loopback:
        try:
            loopback = ipaddress.ip_address(host).is_loopback
        except ValueError:
            pass
    if (endpoint.scheme not in ("http", "https") or not loopback
            or endpoint.username or endpoint.password or endpoint.query or endpoint.fragment):
        raise ValueError("health RPC requires a loopback endpoint without embedded credentials")
    values = {}
    for line in Path(config["credentials_file"]).read_text().splitlines():
        parts = shlex.split(line, comments=True)
        if parts and parts[0] == "export":
            parts = parts[1:]
        if not parts:
            continue
        if len(parts) != 1 or "=" not in parts[0]:
            raise ValueError("credentials file must contain literal environment assignments")
        key, value = parts[0].split("=", 1)
        values[key] = value
    user = values[config["user_key"]]
    password = values[config["password_key"]]
    if not user or not password or ":" in user:
        raise ValueError("invalid health RPC credentials")
    auth = base64.b64encode((user + ":" + password).encode()).decode()
    request = urllib.request.Request(config["endpoint"],
        data=json.dumps({"jsonrpc": "1.0", "id": "mining_health", "method": method, "params": []}).encode(),
        headers={"Authorization": "Basic " + auth, "Content-Type": "application/json"}, method="POST")
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=5) as response:
        result = json.load(response)
    if result.get("error") is not None or not isinstance(result.get("result"), dict):
        raise ValueError("invalid health RPC response")
    return result["result"]


def core_query(config, method):
    if "rpc" in config:
        return rpc_query(config["rpc"], method)
    return json.loads(run(config["bitcoin_cli"] + [method], timeout=5).stdout)


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
    stage = "core_blockchain"
    try:
        blockchain = core_query(config, "getblockchaininfo")
        stage = "core_network"
        network = core_query(config, "getnetworkinfo")
        stage = "miner_api"
        with urllib.request.urlopen(config["miner_url"], timeout=5) as response:
            miner = json.load(response)
        stage = "stratum_tcp"
        tcp = run(["lsof", "-nP", "-iTCP:%s" % config["stratum_port"], "-sTCP:ESTABLISHED"], timeout=5).stdout
        stage = "health_checks"
        ok = healthy(blockchain, network, miner, tcp, config)
        result = {"mining_healthy": bool(ok)}
        if not ok:
            result["failed_check"] = stage
    except Exception as error:
        # Only fixed stage names and exception types are safe to log.
        result = {"mining_healthy": False, "failed_check": stage,
                  "error_type": type(error).__name__}
    print(json.dumps(result))
    return 0 if result["mining_healthy"] else 1


if __name__ == "__main__":
    sys.exit(main())
