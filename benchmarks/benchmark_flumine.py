"""
Benchmark comparing FlumineSimulation (baseline) vs faster_bf-powered simulation.

Based exactly on: https://github.com/betcode-org/flumine/blob/master/examples/simulate.py
"""

import sys
import time
import logging
from pathlib import Path

from pythonjsonlogger import jsonlogger

from flumine import FlumineSimulation, clients
from flumine.streams.historicalstream import HistoricalStream

import faster_bf
from lowestlayer import LowestLayer


# Setup logging exactly as in simulate.py
logger = logging.getLogger()

custom_format = "%(asctime) %(levelname) %(message)"
log_handler = logging.StreamHandler()
formatter = jsonlogger.JsonFormatter(custom_format)
formatter.converter = time.gmtime
log_handler.setFormatter(formatter)
logger.addHandler(log_handler)
logger.setLevel(logging.CRITICAL)  # Set to CRITICAL to speed up simulation

# Market files - canonical flumine test file and our larger test file
FLUMINE_TEST_FILE = Path(__file__).parent / "PRO-1.170258213"
LARGER_TEST_FILE = Path(__file__).parent.parent / "tests" / "data" / "1.241836988.gz"


# Store original create_generator for restoration
_original_create_generator = HistoricalStream.create_generator


def _faster_bf_create_generator(self):
    """Replacement create_generator that uses faster_bf."""
    stream_id = self.stream_id

    def generator():
        for market_book in faster_bf.iter_prices_file(str(self.market_filter)):
            market_book.streaming_unique_id = stream_id
            yield [market_book]

    return generator


def run_baseline_simulation(markets):
    """Run simulation using standard FlumineSimulation - exactly as simulate.py"""
    # Ensure we're using the original create_generator
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

    return framework


def run_faster_bf_simulation(markets):
    """Run simulation using faster_bf-powered stream."""
    # Monkey-patch HistoricalStream to use faster_bf
    HistoricalStream.create_generator = _faster_bf_create_generator

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

    # Restore original
    HistoricalStream.create_generator = _original_create_generator

    return framework


def print_results(framework, label):
    """Print results for a simulation run."""
    for market in framework.markets:
        profit = sum([o.profit for o in market.blotter])
        print(f"{label} Profit: {profit:.2f}")


def benchmark_file(market_file):
    """Run benchmark for a single market file."""
    if not market_file.exists():
        print(f"Market file not found: {market_file}")
        return None

    markets = [str(market_file)]

    print(f"Benchmarking with file: {market_file.name}")
    print(f"File size: {market_file.stat().st_size / 1024 / 1024:.1f} MB")
    print()

    # Warm up faster_bf
    print("Warming up faster_bf...")
    _ = list(faster_bf.iter_prices_file(str(market_file)))

    # Benchmark baseline
    print("Running baseline FlumineSimulation...")
    start = time.perf_counter()
    baseline_framework = run_baseline_simulation(markets)
    baseline_time = time.perf_counter() - start
    print(f"  Baseline time: {baseline_time:.2f}s")
    print_results(baseline_framework, "  Baseline")

    # Benchmark faster_bf
    print("Running faster_bf-powered simulation...")
    start = time.perf_counter()
    faster_bf_framework = run_faster_bf_simulation(markets)
    faster_bf_time = time.perf_counter() - start
    print(f"  faster_bf time: {faster_bf_time:.2f}s")
    print_results(faster_bf_framework, "  faster_bf")

    # Results
    speedup = baseline_time / faster_bf_time
    print()
    print("=" * 50)
    print("BENCHMARK RESULTS")
    print("=" * 50)
    print(f"Baseline:   {baseline_time:.2f}s")
    print(f"faster_bf:  {faster_bf_time:.2f}s")
    print(f"Speedup:    {speedup:.2f}x")
    print("=" * 50)
    return speedup


def main():
    # Check for command line argument to select file
    if len(sys.argv) > 1 and sys.argv[1] == "--large":
        benchmark_file(LARGER_TEST_FILE)
    else:
        benchmark_file(FLUMINE_TEST_FILE)


if __name__ == "__main__":
    main()
