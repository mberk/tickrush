"""
Benchmark comparing FlumineSimulation baseline (betfairlightweight) vs tickrush.

Based on: https://github.com/betcode-org/flumine/blob/master/examples/simulate.py
"""

import sys
import time
import logging
from pathlib import Path

from pythonjsonlogger import jsonlogger

from flumine import FlumineSimulation, clients
from flumine.streams.historicalstream import HistoricalStream

import tickrush
from lowestlayer import LowestLayer


# Suppress logging to avoid benchmark noise
logger = logging.getLogger()
custom_format = "%(asctime) %(levelname) %(message)"
log_handler = logging.StreamHandler()
formatter = jsonlogger.JsonFormatter(custom_format)
formatter.converter = time.gmtime
log_handler.setFormatter(formatter)
logger.addHandler(log_handler)
logger.setLevel(logging.CRITICAL)

TEST_FILE = Path(__file__).parent.parent / "tests" / "resources" / "PRO-1.170258213"

# Store original create_generator for restoration
_original_create_generator = HistoricalStream.create_generator


def run_simulation(markets, use_tickrush=False):
    """Run a flumine simulation, optionally using tickrush."""
    if use_tickrush:
        HistoricalStream.create_generator = tickrush.create_generator
    else:
        HistoricalStream.create_generator = _original_create_generator

    client = clients.SimulatedClient()
    framework = FlumineSimulation(client=client)

    strategy = LowestLayer(
        market_filter={"markets": markets},
        max_order_exposure=1000,
        max_selection_exposure=105,
        context={"stake": 2},
    )
    framework.add_strategy(strategy)
    framework.run()

    # Always restore
    HistoricalStream.create_generator = _original_create_generator
    return framework


def get_profit(framework):
    """Get total profit from a simulation run."""
    total = 0.0
    for market in framework.markets:
        total += sum(o.profit for o in market.blotter)
    return total


def main():
    if not TEST_FILE.exists():
        print(f"Test file not found: {TEST_FILE}")
        sys.exit(1)

    markets = [str(TEST_FILE)]
    print(f"File: {TEST_FILE.name} ({TEST_FILE.stat().st_size / 1024 / 1024:.1f} MB)")

    # Run baseline
    print("Running baseline (betfairlightweight)...")
    start = time.perf_counter()
    baseline = run_simulation(markets, use_tickrush=False)
    baseline_time = time.perf_counter() - start
    baseline_profit = get_profit(baseline)
    print(f"  Time: {baseline_time:.2f}s  Profit: {baseline_profit:.2f}")

    # Run tickrush
    print("Running tickrush...")
    start = time.perf_counter()
    tickrush_fw = run_simulation(markets, use_tickrush=True)
    tickrush_time = time.perf_counter() - start
    tickrush_profit = get_profit(tickrush_fw)
    print(f"  Time: {tickrush_time:.2f}s  Profit: {tickrush_profit:.2f}")

    # Results
    speedup = baseline_time / tickrush_time
    profit_match = "MATCH" if abs(baseline_profit - tickrush_profit) < 0.01 else "MISMATCH"
    print()
    print(f"Baseline:  {baseline_time:.2f}s")
    print(f"Tickrush:  {tickrush_time:.2f}s")
    print(f"Speedup:   {speedup:.2f}x")
    print(f"Profit:    {profit_match} (baseline={baseline_profit:.2f}, tickrush={tickrush_profit:.2f})")


if __name__ == "__main__":
    main()
