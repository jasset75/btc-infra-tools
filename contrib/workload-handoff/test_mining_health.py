import unittest
from mining_health import healthy


class MiningHealthTests(unittest.TestCase):
    def test_all_protected_checks_are_required(self):
        block = dict(blocks=12, headers=12, initialblockdownload=False)
        net = dict(networkactive=True, connections=5)
        miner = dict(isUsingFallbackStratum=0, stratumURL="primary", stratumPort=3333, hashRate=1000)
        config = dict(stratum_host="primary", stratum_port=3333, tcp_peer="primary:3333->miner:")
        tcp = "primary:3333->miner:6000 (ESTABLISHED)"
        self.assertTrue(healthy(block, net, miner, tcp, config))
        for obj, key, value in [(block, "headers", 13), (block, "initialblockdownload", True),
                                (net, "networkactive", False), (net, "connections", 0),
                                (miner, "isUsingFallbackStratum", 1), (miner, "stratumURL", "fallback"),
                                (miner, "stratumPort", 3334), (miner, "hashRate", 0)]:
            old = obj[key]
            obj[key] = value
            self.assertFalse(healthy(block, net, miner, tcp, config))
            obj[key] = old
        self.assertFalse(healthy(block, net, miner, "", config))


if __name__ == "__main__":
    unittest.main()
