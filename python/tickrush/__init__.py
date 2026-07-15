import datetime

from .tickrush import (
    ExchangePrices,
    MarketBook,
    MarketDefinition,
    MarketDefinitionRunner,
    PricesIterator,
    PriceSize,
    Runner,
    StartingPrices,
    iter_prices_file,
)

__version__ = "0.1.0"
__all__ = [
    "iter_prices_file",
    "create_generator",
    "MarketBook",
    "MarketDefinition",
    "MarketDefinitionRunner",
    "Runner",
    "ExchangePrices",
    "StartingPrices",
    "PriceSize",
    "PricesIterator",
]


def create_generator(self):
    """Drop-in replacement for
    `flumine.streams.historicalstream.HistoricalStream.create_generator`.

    Assign as `HistoricalStream.create_generator = tickrush.create_generator`
    to have `flumine` read historical price files via tickrush instead of
    betfairlightweight. Mirrors the filtering behaviour of
    `flumine.streams.historicalstream.FlumineMarketStream._process`
    (`inplay`, `seconds_to_start` and `max_inplay_seconds` listener kwargs)
    so simulation results match the unpatched code path.
    """
    stream_id = self.stream_id
    file_path = self.market_filter
    listener = self._listener
    listener.update_clk = False

    def generator():
        inplay_publish_time = None
        was_in_play = False

        for market_book in iter_prices_file(str(file_path)):
            market_book.streaming_unique_id = stream_id

            status = market_book.status
            in_play = market_book.inplay
            publish_time_epoch = market_book.publish_time_epoch

            if listener.max_inplay_seconds is not None and in_play and not was_in_play:
                inplay_publish_time = publish_time_epoch
            was_in_play = in_play

            active = True
            if status == "OPEN":
                if listener.inplay:
                    if not in_play:
                        active = False
                elif listener.seconds_to_start is not None:
                    definition = market_book.market_definition
                    market_time = definition.market_time if definition else None
                    if market_time is not None:
                        now = datetime.datetime.fromtimestamp(
                            publish_time_epoch / 1e3, tz=datetime.timezone.utc
                        )
                        seconds_to_start = (market_time - now).total_seconds()
                        if seconds_to_start > listener.seconds_to_start:
                            active = False
                if listener.inplay is False and in_play:
                    active = False
                if (
                    listener.max_inplay_seconds is not None
                    and inplay_publish_time is not None
                ):
                    inplay_seconds = (publish_time_epoch - inplay_publish_time) / 1000
                    if inplay_seconds > listener.max_inplay_seconds:
                        active = False

            if active:
                yield [market_book]

    return generator
