from pathlib import Path

import betfairutil as bu
import tickrush

TEST_FILE = Path(__file__).resolve().parent / "resources" / "PRO-1.170258213"

# Keys to compare at the top level of the market book dict
MARKET_KEYS = [
    "marketId", "status", "betDelay", "bspReconciled", "complete",
    "crossMatching", "inplay", "numberOfActiveRunners", "numberOfRunners",
    "numberOfWinners", "runnersVoidable", "totalMatched", "version",
    "publishTime",
]

# Keys to compare for the market definition
DEF_KEYS = [
    "betDelay", "bettingType", "bspMarket", "bspReconciled", "complete",
    "countryCode", "crossMatching", "discountAllowed", "eventId", "eventName",
    "eventTypeId", "inPlay", "marketBaseRate", "marketTime", "marketType",
    "name", "numberOfActiveRunners", "numberOfWinners", "openDate",
    "persistenceEnabled", "regulators", "runnersVoidable", "status",
    "suspendTime", "timezone", "turnInPlayEnabled", "venue", "version",
]

# Round floats to 10 decimal places to handle cross-parser (Python json vs Rust serde_json)
# precision differences on BSP prices (e.g. 12.132481138665863 vs 12.132481138665865)
FLOAT_PRECISION = 10


def rps(ps_list):
    """Round price/size dicts to consistent precision."""
    return [
        {"price": round(ps["price"], FLOAT_PRECISION), "size": round(ps["size"], FLOAT_PRECISION)}
        for ps in ps_list
    ]


def normalize_runner(r):
    """Extract comparable fields from a betfairutil lightweight runner dict."""
    return {
        "selectionId": r["selectionId"],
        "handicap": float(r["handicap"]),
        "status": r.get("status"),
        "adjustmentFactor": r.get("adjustmentFactor"),
        "lastPriceTraded": r.get("lastPriceTraded"),
        "totalMatched": float(r["totalMatched"]),
        "removalDate": r.get("removalDate"),
        "ex": {
            "availableToBack": rps(r["ex"]["availableToBack"]),
            "availableToLay": rps(r["ex"]["availableToLay"]),
            "tradedVolume": rps(r["ex"]["tradedVolume"]),
        },
        "sp": {
            "nearPrice": r["sp"]["nearPrice"],
            "farPrice": r["sp"]["farPrice"],
            "actualSP": r["sp"]["actualSP"],
            "backStakeTaken": rps(r["sp"]["backStakeTaken"]),
            "layLiabilityTaken": rps(r["sp"]["layLiabilityTaken"]),
        },
    }


def normalize_actual_runner(r):
    """Round price/size floats in a tickrush runner dict."""
    r["ex"]["availableToBack"] = rps(r["ex"]["availableToBack"])
    r["ex"]["availableToLay"] = rps(r["ex"]["availableToLay"])
    r["ex"]["tradedVolume"] = rps(r["ex"]["tradedVolume"])
    r["sp"]["backStakeTaken"] = rps(r["sp"]["backStakeTaken"])
    r["sp"]["layLiabilityTaken"] = rps(r["sp"]["layLiabilityTaken"])
    return r


def normalize_def(md):
    """Extract comparable fields from a betfairutil lightweight market definition."""
    d = {k: md.get(k) for k in DEF_KEYS}
    d["regulators"] = md.get("regulators", [])
    if "runners" in md:
        d["runners"] = sorted(
            [{k: r[k] for k in r if k not in ("ex",)} for r in md["runners"]],
            key=lambda r: r["id"],
        )
    return d


def normalize(mb):
    """Extract comparable fields from a betfairutil lightweight market book dict."""
    d = {k: mb[k] for k in MARKET_KEYS}
    d["runners"] = sorted(
        [normalize_runner(r) for r in mb["runners"]],
        key=lambda r: (r["selectionId"], r["handicap"]),
    )
    if "marketDefinition" in mb and mb["marketDefinition"] is not None:
        d["marketDefinition"] = normalize_def(mb["marketDefinition"])
    return d


def normalize_actual(mb_dict):
    """Normalize a tickrush dict for comparison."""
    d = {k: mb_dict[k] for k in MARKET_KEYS}
    d["runners"] = sorted(
        [normalize_actual_runner(r) for r in mb_dict["runners"]],
        key=lambda r: (r["selectionId"], r["handicap"]),
    )
    if "marketDefinition" in mb_dict and mb_dict["marketDefinition"] is not None:
        md = mb_dict["marketDefinition"]
        d["marketDefinition"] = {k: md.get(k) for k in DEF_KEYS}
        d["marketDefinition"]["regulators"] = md.get("regulators", [])
        if "runners" in md:
            d["marketDefinition"]["runners"] = sorted(
                md["runners"],
                key=lambda r: r["id"],
            )
    return d


def test_correctness():
    for i, (expected_dict, actual) in enumerate(zip(
        bu.create_market_book_generator_from_prices_file(TEST_FILE, lightweight=True),
        tickrush.iter_prices_file(str(TEST_FILE)),
    )):
        expected = normalize(expected_dict)
        actual_d = normalize_actual(actual.to_dict())
        assert expected == actual_d, f"Mismatch at message {i}"
