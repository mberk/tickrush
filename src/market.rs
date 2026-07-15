//! Market data structures with #[pyclass] for efficient Python interop.
//!
//! Key optimization: We cache Py<T> references and only create new Python objects
//! when the underlying data actually changes (tracked via generation counters).

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use serde_json::Value;
use std::collections::HashMap;

// =============================================================================
// Helpers
// =============================================================================

/// Parse a Betfair datetime string like "2024-01-15T14:30:00.000Z" into a
/// timezone-aware (UTC) Python datetime (matching current betfairlightweight,
/// which attaches tzinfo=timezone.utc via ciso8601/strptime).
fn parse_datetime(py: Python<'_>, s: &str) -> PyResult<PyObject> {
    let datetime_mod = py.import_bound("datetime")?;
    let datetime_cls = datetime_mod.getattr("datetime")?;
    let utc = datetime_mod.getattr("timezone")?.getattr("utc")?;

    // Try parsing with fractional seconds first, then without
    let result = datetime_cls.call_method1("strptime", (s, "%Y-%m-%dT%H:%M:%S.%fZ"));
    let dt = match result {
        Ok(dt) => dt,
        Err(_) => datetime_cls.call_method1("strptime", (s, "%Y-%m-%dT%H:%M:%SZ"))?,
    };
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("tzinfo", &utc)?;
    let dt_aware = dt.call_method("replace", (), Some(&kwargs))?;
    Ok(dt_aware.into())
}

/// Parse a datetime string to PyObject, or return py.None()
fn parse_datetime_or_none(py: Python<'_>, s: &Option<String>) -> PyResult<PyObject> {
    match s {
        Some(ref s) => parse_datetime(py, s),
        None => Ok(py.None()),
    }
}

// =============================================================================
// Python-exposed classes (immutable views into Rust data)
// =============================================================================

/// A price/size pair in the order book
#[pyclass(frozen, get_all)]
#[derive(Clone)]
pub struct PriceSize {
    pub price: f64,
    pub size: f64,
}

#[pymethods]
impl PriceSize {
    fn __repr__(&self) -> String {
        format!("PriceSize(price={}, size={})", self.price, self.size)
    }

    fn __getitem__(&self, key: &str) -> PyResult<f64> {
        match key {
            "price" => Ok(self.price),
            "size" => Ok(self.size),
            _ => Err(pyo3::exceptions::PyKeyError::new_err(key.to_string())),
        }
    }
}

/// Exchange prices for a runner (back/lay/traded)
#[pyclass]
pub struct ExchangePrices {
    pub(crate) cached_back: PyObject,
    pub(crate) cached_lay: PyObject,
    pub(crate) cached_traded: PyObject,
}

#[pymethods]
impl ExchangePrices {
    #[getter]
    fn available_to_back(&self, py: Python<'_>) -> PyObject {
        self.cached_back.clone_ref(py)
    }

    #[getter]
    fn available_to_lay(&self, py: Python<'_>) -> PyObject {
        self.cached_lay.clone_ref(py)
    }

    #[getter]
    fn traded_volume(&self, py: Python<'_>) -> PyObject {
        self.cached_traded.clone_ref(py)
    }
}

/// Starting prices for a runner
#[pyclass(frozen)]
#[derive(Clone)]
pub struct StartingPrices {
    pub(crate) near_price: SpValue,
    pub(crate) far_price: SpValue,
    pub(crate) actual_sp: SpValue,
    pub(crate) back_stake_taken: Vec<PriceSize>,
    pub(crate) lay_liability_taken: Vec<PriceSize>,
}

/// SP value: can be f64, string ("Infinity"), or None
#[derive(Clone)]
pub(crate) enum SpValue {
    Float(f64),
    Str(String),
    None,
}

impl SpValue {
    fn from_json(v: &Value) -> Self {
        if let Some(f) = v.as_f64() {
            SpValue::Float(f)
        } else if let Some(s) = v.as_str() {
            SpValue::Str(s.to_string())
        } else {
            SpValue::None
        }
    }

    fn to_pyobject(&self, py: Python<'_>) -> PyObject {
        match self {
            SpValue::Float(f) => f.into_py(py),
            SpValue::Str(s) => s.into_py(py),
            SpValue::None => py.None(),
        }
    }
}

#[pymethods]
impl StartingPrices {
    #[getter] fn near_price(&self, py: Python<'_>) -> PyObject { self.near_price.to_pyobject(py) }
    #[getter] fn far_price(&self, py: Python<'_>) -> PyObject { self.far_price.to_pyobject(py) }
    #[getter] fn actual_sp(&self, py: Python<'_>) -> PyObject { self.actual_sp.to_pyobject(py) }
    #[getter] fn back_stake_taken(&self) -> Vec<PriceSize> { self.back_stake_taken.clone() }
    #[getter] fn lay_liability_taken(&self) -> Vec<PriceSize> { self.lay_liability_taken.clone() }
}

/// A runner in a market definition
#[pyclass(frozen, get_all)]
#[derive(Clone)]
pub struct MarketDefinitionRunner {
    pub selection_id: i64,
    pub sort_priority: i32,
    pub status: String,
    pub handicap: f64,
    pub adjustment_factor: Option<f64>,
    pub removal_date: Option<String>,
    pub name: Option<String>,
    pub bsp: Option<f64>,
}

/// Market definition - metadata about the market
#[pyclass(frozen)]
pub struct MarketDefinition {
    pub(crate) event_id: Option<String>,
    pub(crate) event_type_id: Option<String>,
    pub(crate) event_name: Option<String>,
    pub(crate) market_type: Option<String>,
    pub(crate) country_code: Option<String>,
    pub(crate) venue: Option<String>,
    pub(crate) race_type: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) timezone: Option<String>,
    pub(crate) betting_type: Option<String>,
    pub(crate) regulators: Vec<String>,
    pub(crate) cached_runners: PyObject,       // Python list of MarketDefinitionRunner
    pub(crate) cached_market_time: PyObject,   // datetime or None
    pub(crate) cached_open_date: PyObject,
    pub(crate) cached_suspend_time: PyObject,
    pub(crate) cached_settled_time: PyObject,
    // Raw datetime strings for to_dict() (avoids re-formatting cached datetimes)
    pub(crate) raw_market_time: Option<String>,
    pub(crate) raw_open_date: Option<String>,
    pub(crate) raw_suspend_time: Option<String>,
    pub(crate) bsp_market: bool,
    pub(crate) persistence_enabled: bool,
    pub(crate) each_way_divisor: Option<f64>,
    pub(crate) turn_in_play_enabled: bool,
    pub(crate) discount_allowed: bool,
    pub(crate) market_base_rate: f64,
    pub(crate) bet_delay: i32,
    pub(crate) number_of_winners: i32,
    pub(crate) number_of_active_runners: i32,
    pub(crate) bsp_reconciled: bool,
    pub(crate) complete: bool,
    pub(crate) cross_matching: bool,
    pub(crate) in_play: bool,
    pub(crate) runners_voidable: bool,
    pub(crate) status: String,
    pub(crate) version: i64,
}

#[pymethods]
impl MarketDefinition {
    #[getter] fn event_id(&self) -> Option<&str> { self.event_id.as_deref() }
    #[getter] fn event_type_id(&self) -> Option<&str> { self.event_type_id.as_deref() }
    #[getter] fn event_name(&self) -> Option<&str> { self.event_name.as_deref() }
    #[getter] fn market_type(&self) -> Option<&str> { self.market_type.as_deref() }
    #[getter] fn country_code(&self) -> Option<&str> { self.country_code.as_deref() }
    #[getter] fn venue(&self) -> Option<&str> { self.venue.as_deref() }
    #[getter] fn race_type(&self) -> Option<&str> { self.race_type.as_deref() }
    #[getter] fn name(&self) -> Option<&str> { self.name.as_deref() }
    #[getter] fn timezone(&self) -> Option<&str> { self.timezone.as_deref() }
    #[getter] fn betting_type(&self) -> Option<&str> { self.betting_type.as_deref() }
    #[getter] fn regulators(&self) -> Vec<String> { self.regulators.clone() }
    #[getter] fn runners(&self, py: Python<'_>) -> PyObject { self.cached_runners.clone_ref(py) }
    #[getter] fn market_time(&self, py: Python<'_>) -> PyObject { self.cached_market_time.clone_ref(py) }
    #[getter] fn open_date(&self, py: Python<'_>) -> PyObject { self.cached_open_date.clone_ref(py) }
    #[getter] fn suspend_time(&self, py: Python<'_>) -> PyObject { self.cached_suspend_time.clone_ref(py) }
    #[getter] fn settled_time(&self, py: Python<'_>) -> PyObject { self.cached_settled_time.clone_ref(py) }
    #[getter] fn bsp_market(&self) -> bool { self.bsp_market }
    #[getter] fn persistence_enabled(&self) -> bool { self.persistence_enabled }
    #[getter] fn each_way_divisor(&self) -> Option<f64> { self.each_way_divisor }
    #[getter] fn turn_in_play_enabled(&self) -> bool { self.turn_in_play_enabled }
    #[getter] fn discount_allowed(&self) -> bool { self.discount_allowed }
    #[getter] fn market_base_rate(&self) -> f64 { self.market_base_rate }
    #[getter] fn bet_delay(&self) -> i32 { self.bet_delay }
    #[getter] fn number_of_winners(&self) -> i32 { self.number_of_winners }
    #[getter] fn number_of_active_runners(&self) -> i32 { self.number_of_active_runners }
    #[getter] fn bsp_reconciled(&self) -> bool { self.bsp_reconciled }
    #[getter] fn complete(&self) -> bool { self.complete }
    #[getter] fn cross_matching(&self) -> bool { self.cross_matching }
    #[getter] fn in_play(&self) -> bool { self.in_play }
    #[getter] fn runners_voidable(&self) -> bool { self.runners_voidable }
    #[getter] fn status(&self) -> &str { &self.status }
    #[getter] fn version(&self) -> i64 { self.version }

    // Fields that are always None in streaming context
    #[getter] fn bet_delay_models(&self) -> Option<bool> { None }
    #[getter] fn key_line_definitions(&self) -> Option<bool> { None }
    #[getter] fn line_interval(&self) -> Option<f64> { None }
    #[getter] fn line_max_unit(&self) -> Option<f64> { None }
    #[getter] fn line_min_unit(&self) -> Option<f64> { None }
    #[getter] fn price_ladder_definition(&self) -> Option<bool> { None }
    #[getter] fn suspend_reason(&self) -> Option<&str> { None }
}

/// A runner's live order book within a MarketBook
#[pyclass(frozen)]
pub struct RunnerBook {
    #[pyo3(get)]
    pub(crate) selection_id: i64,
    #[pyo3(get)]
    pub(crate) handicap: f64,
    pub(crate) cached_status: PyObject,
    #[pyo3(get)]
    pub(crate) adjustment_factor: Option<f64>,
    #[pyo3(get)]
    pub(crate) last_price_traded: Option<f64>,
    #[pyo3(get)]
    pub(crate) total_matched: f64,
    pub(crate) removal_date: Option<String>,
    pub(crate) ex: Py<ExchangePrices>,
    pub(crate) sp: StartingPrices,
}

#[pymethods]
impl RunnerBook {
    #[getter]
    fn status(&self, py: Python<'_>) -> PyObject {
        self.cached_status.clone_ref(py)
    }

    #[getter]
    fn removal_date(&self) -> Option<&str> {
        self.removal_date.as_deref()
    }

    #[getter]
    fn ex(&self, py: Python<'_>) -> PyObject {
        self.ex.clone_ref(py).into_py(py)
    }

    #[getter]
    fn sp(&self) -> StartingPrices {
        self.sp.clone()
    }

    // Fields that are always None/empty in streaming context
    #[getter] fn orders(&self) -> Vec<bool> { Vec::new() }
    #[getter] fn matches(&self) -> Vec<bool> { Vec::new() }
    #[getter] fn matches_by_strategy(&self) -> Option<bool> { None }
    #[getter] fn bet_delay_models(&self) -> Option<bool> { None }
    #[getter] fn suspend_reason(&self) -> Option<&str> { None }
}

/// A market book snapshot
#[pyclass]
pub struct MarketBook {
    pub(crate) market_id: String,
    pub(crate) publish_time_millis: i64,
    pub(crate) cached_publish_time: PyObject,
    pub(crate) cached_runners: PyObject,
    pub(crate) status: String,
    pub(crate) bet_delay: i32,
    pub(crate) bsp_reconciled: bool,
    pub(crate) complete: bool,
    pub(crate) cross_matching: bool,
    pub(crate) inplay: bool,
    pub(crate) number_of_active_runners: i32,
    pub(crate) number_of_runners: i32,
    pub(crate) number_of_winners: i32,
    pub(crate) runners_voidable: bool,
    pub(crate) total_matched: f64,
    pub(crate) version: i64,
    pub(crate) last_match_time: Option<i64>,
    pub(crate) market_definition: Option<Py<MarketDefinition>>,
    /// Streaming unique ID (set by stream for flumine compatibility)
    #[pyo3(get, set)]
    pub streaming_unique_id: Option<i32>,
}

#[pymethods]
impl MarketBook {
    #[getter]
    fn market_id(&self) -> &str {
        &self.market_id
    }

    #[getter]
    fn publish_time(&self, py: Python<'_>) -> PyObject {
        self.cached_publish_time.clone_ref(py)
    }

    /// publish_time in epoch milliseconds (int, for flumine compatibility)
    #[getter]
    fn publish_time_epoch(&self) -> i64 {
        self.publish_time_millis
    }

    #[getter]
    fn status(&self) -> &str {
        &self.status
    }

    #[getter]
    fn bet_delay(&self) -> i32 {
        self.bet_delay
    }

    #[getter]
    fn bsp_reconciled(&self) -> bool {
        self.bsp_reconciled
    }

    #[getter]
    fn complete(&self) -> bool {
        self.complete
    }

    #[getter]
    fn cross_matching(&self) -> bool {
        self.cross_matching
    }

    #[getter]
    fn inplay(&self) -> bool {
        self.inplay
    }

    #[getter]
    fn number_of_active_runners(&self) -> i32 {
        self.number_of_active_runners
    }

    #[getter]
    fn number_of_runners(&self) -> i32 {
        self.number_of_runners
    }

    #[getter]
    fn number_of_winners(&self) -> i32 {
        self.number_of_winners
    }

    #[getter]
    fn runners_voidable(&self) -> bool {
        self.runners_voidable
    }

    #[getter]
    fn total_matched(&self) -> f64 {
        self.total_matched
    }

    #[getter]
    fn version(&self) -> i64 {
        self.version
    }

    #[getter]
    fn last_match_time(&self) -> Option<i64> {
        self.last_match_time
    }

    #[getter]
    fn market_definition(&self, py: Python<'_>) -> Option<PyObject> {
        self.market_definition.as_ref().map(|d| d.clone_ref(py).into_py(py))
    }

    #[getter]
    fn runners(&self, py: Python<'_>) -> PyObject {
        self.cached_runners.clone_ref(py)
    }

    // Fields that are always None in streaming context
    #[getter] fn bet_delay_models(&self) -> Option<bool> { None }
    #[getter] fn elapsed_time(&self) -> Option<bool> { None }
    #[getter] fn is_market_data_delayed(&self) -> Option<bool> { None }
    #[getter] fn key_line_description(&self) -> Option<bool> { None }
    #[getter] fn price_ladder_definition(&self) -> Option<bool> { None }
    #[getter] fn suspend_reason(&self) -> Option<&str> { None }
    #[getter] fn total_available(&self) -> Option<f64> { None }
    #[getter] fn streaming_snap(&self) -> bool { true }
    #[getter] fn streaming_update(&self) -> Option<bool> { None }

    /// Convert to a dict matching betfairutil lightweight format (for testing)
    fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        let dict = PyDict::new_bound(py);
        dict.set_item("marketId", &self.market_id)?;
        dict.set_item("status", &self.status)?;
        dict.set_item("betDelay", self.bet_delay)?;
        dict.set_item("bspReconciled", self.bsp_reconciled)?;
        dict.set_item("complete", self.complete)?;
        dict.set_item("crossMatching", self.cross_matching)?;
        dict.set_item("inplay", self.inplay)?;
        dict.set_item("numberOfActiveRunners", self.number_of_active_runners)?;
        dict.set_item("numberOfRunners", self.number_of_runners)?;
        dict.set_item("numberOfWinners", self.number_of_winners)?;
        dict.set_item("runnersVoidable", self.runners_voidable)?;
        dict.set_item("totalMatched", self.total_matched)?;
        dict.set_item("version", self.version)?;
        dict.set_item("lastMatchTime", self.last_match_time)?;
        dict.set_item("publishTime", self.publish_time_millis)?;

        // Runners
        let runners_list = self.cached_runners.bind(py).downcast::<PyList>()
            .map_err(|e| pyo3::exceptions::PyTypeError::new_err(e.to_string()))?;
        let runner_dicts = PyList::empty_bound(py);
        for runner_obj in runners_list.iter() {
            let runner: PyRef<RunnerBook> = runner_obj.extract()?;
            let r_dict = PyDict::new_bound(py);
            r_dict.set_item("selectionId", runner.selection_id)?;
            r_dict.set_item("handicap", runner.handicap)?;
            r_dict.set_item("status", runner.cached_status.clone_ref(py))?;
            r_dict.set_item("adjustmentFactor", runner.adjustment_factor)?;
            r_dict.set_item("lastPriceTraded", runner.last_price_traded)?;
            r_dict.set_item("totalMatched", runner.total_matched)?;
            r_dict.set_item("removalDate", runner.removal_date.as_deref())?;

            // Exchange prices
            let ex_bound = runner.ex.bind(py);
            let ex = ex_bound.borrow();
            let ex_dict = PyDict::new_bound(py);
            ex_dict.set_item("availableToBack", price_list_to_dicts(py, &ex.cached_back)?)?;
            ex_dict.set_item("availableToLay", price_list_to_dicts(py, &ex.cached_lay)?)?;
            ex_dict.set_item("tradedVolume", price_list_to_dicts(py, &ex.cached_traded)?)?;
            r_dict.set_item("ex", ex_dict)?;

            // Starting prices
            let sp_dict = PyDict::new_bound(py);
            sp_dict.set_item("nearPrice", runner.sp.near_price.to_pyobject(py))?;
            sp_dict.set_item("farPrice", runner.sp.far_price.to_pyobject(py))?;
            sp_dict.set_item("actualSP", runner.sp.actual_sp.to_pyobject(py))?;
            sp_dict.set_item("backStakeTaken", vec_ps_to_dicts(py, &runner.sp.back_stake_taken)?)?;
            sp_dict.set_item("layLiabilityTaken", vec_ps_to_dicts(py, &runner.sp.lay_liability_taken)?)?;
            r_dict.set_item("sp", sp_dict)?;

            runner_dicts.append(r_dict)?;
        }
        dict.set_item("runners", runner_dicts)?;

        // Market definition
        if let Some(ref md_py) = self.market_definition {
            let md = md_py.bind(py).borrow();
            let md_dict = PyDict::new_bound(py);
            md_dict.set_item("betDelay", md.bet_delay)?;
            md_dict.set_item("bettingType", md.betting_type.as_deref())?;
            md_dict.set_item("bspMarket", md.bsp_market)?;
            md_dict.set_item("bspReconciled", md.bsp_reconciled)?;
            md_dict.set_item("complete", md.complete)?;
            md_dict.set_item("countryCode", md.country_code.as_deref())?;
            md_dict.set_item("crossMatching", md.cross_matching)?;
            md_dict.set_item("discountAllowed", md.discount_allowed)?;
            md_dict.set_item("eventId", md.event_id.as_deref())?;
            md_dict.set_item("eventName", md.event_name.as_deref())?;
            md_dict.set_item("eventTypeId", md.event_type_id.as_deref())?;
            md_dict.set_item("inPlay", md.in_play)?;
            md_dict.set_item("marketBaseRate", md.market_base_rate)?;
            md_dict.set_item("marketTime", md.raw_market_time.as_deref())?;
            md_dict.set_item("marketType", md.market_type.as_deref())?;
            md_dict.set_item("name", md.name.as_deref())?;
            md_dict.set_item("numberOfActiveRunners", md.number_of_active_runners)?;
            md_dict.set_item("numberOfWinners", md.number_of_winners)?;
            md_dict.set_item("openDate", md.raw_open_date.as_deref())?;
            md_dict.set_item("persistenceEnabled", md.persistence_enabled)?;
            let regs = PyList::new_bound(py, md.regulators.iter().map(|s| s.as_str()));
            md_dict.set_item("regulators", regs)?;

            // Definition runners
            let md_runners = md.cached_runners.bind(py).downcast::<PyList>()
                .map_err(|e| pyo3::exceptions::PyTypeError::new_err(e.to_string()))?;
            let md_runner_dicts = PyList::empty_bound(py);
            for r_obj in md_runners.iter() {
                let r: PyRef<MarketDefinitionRunner> = r_obj.extract()?;
                let rd = PyDict::new_bound(py);
                rd.set_item("id", r.selection_id)?;
                rd.set_item("sortPriority", r.sort_priority)?;
                rd.set_item("status", &r.status)?;
                if r.adjustment_factor.is_some() {
                    rd.set_item("adjustmentFactor", r.adjustment_factor)?;
                }
                if r.name.is_some() {
                    rd.set_item("name", r.name.as_deref())?;
                }
                if r.handicap != 0.0 {
                    rd.set_item("hc", r.handicap)?;
                }
                if r.removal_date.is_some() {
                    rd.set_item("removalDate", r.removal_date.as_deref())?;
                }
                if r.bsp.is_some() {
                    rd.set_item("bsp", r.bsp)?;
                }
                md_runner_dicts.append(rd)?;
            }
            md_dict.set_item("runners", md_runner_dicts)?;

            md_dict.set_item("runnersVoidable", md.runners_voidable)?;
            md_dict.set_item("status", &md.status)?;
            md_dict.set_item("suspendTime", md.raw_suspend_time.as_deref())?;
            md_dict.set_item("timezone", md.timezone.as_deref())?;
            md_dict.set_item("turnInPlayEnabled", md.turn_in_play_enabled)?;
            md_dict.set_item("venue", md.venue.as_deref())?;
            md_dict.set_item("version", md.version)?;
            dict.set_item("marketDefinition", md_dict)?;
        }

        Ok(dict.into())
    }
}

/// Convert a Python list of PriceSize objects to a list of {price, size} dicts
fn price_list_to_dicts<'py>(py: Python<'py>, list_obj: &PyObject) -> PyResult<Bound<'py, PyList>> {
    let list = list_obj.bind(py).downcast::<PyList>()
        .map_err(|e| pyo3::exceptions::PyTypeError::new_err(e.to_string()))?;
    let result = PyList::empty_bound(py);
    for item in list.iter() {
        let ps: PyRef<PriceSize> = item.extract()?;
        let d = PyDict::new_bound(py);
        d.set_item("price", ps.price)?;
        d.set_item("size", ps.size)?;
        result.append(d)?;
    }
    Ok(result)
}

/// Convert a slice of PriceSize to a list of {price, size} dicts
fn vec_ps_to_dicts<'py>(py: Python<'py>, items: &[PriceSize]) -> PyResult<Bound<'py, PyList>> {
    let result = PyList::empty_bound(py);
    for ps in items {
        let d = PyDict::new_bound(py);
        d.set_item("price", ps.price)?;
        d.set_item("size", ps.size)?;
        result.append(d)?;
    }
    Ok(result)
}

// =============================================================================
// Internal cache structures (Rust-side, with generation tracking)
// =============================================================================

/// Price ladder entry (internal)
/// Uses f64::to_bits() as key to preserve full precision (needed for BSP prices)
struct PriceLadder {
    prices: HashMap<u64, f64>,
    reverse: bool,
    generation: u64,
}

impl PriceLadder {
    fn new(reverse: bool) -> Self {
        PriceLadder {
            prices: HashMap::new(),
            reverse,
            generation: 0,
        }
    }

    fn update(&mut self, updates: &[Value]) -> bool {
        let mut changed = false;
        for update in updates {
            if let Some(arr) = update.as_array() {
                if arr.len() >= 2 {
                    let price = arr[0].as_f64().unwrap_or(0.0);
                    let size = arr[1].as_f64().unwrap_or(0.0);
                    let price_key = price.to_bits();

                    if size == 0.0 {
                        if self.prices.remove(&price_key).is_some() {
                            changed = true;
                        }
                    } else {
                        let old = self.prices.insert(price_key, size);
                        if old != Some(size) {
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.generation += 1;
        }
        changed
    }

    fn to_price_sizes(&self) -> Vec<PriceSize> {
        let mut items: Vec<_> = self
            .prices
            .iter()
            .map(|(&k, &v)| (f64::from_bits(k), v))
            .collect();

        if self.reverse {
            items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        } else {
            items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        }

        items
            .into_iter()
            .map(|(price, size)| PriceSize { price, size })
            .collect()
    }
}

/// Traded volume ladder (internal)
struct TradedLadder {
    prices: HashMap<u64, f64>,
    generation: u64,
}

impl TradedLadder {
    fn new() -> Self {
        TradedLadder {
            prices: HashMap::new(),
            generation: 0,
        }
    }

    fn update(&mut self, updates: &[Value]) -> bool {
        let mut changed = false;
        for update in updates {
            if let Some(arr) = update.as_array() {
                if arr.len() >= 2 {
                    let price = arr[0].as_f64().unwrap_or(0.0);
                    let size = arr[1].as_f64().unwrap_or(0.0);
                    let price_key = price.to_bits();

                    if size == 0.0 {
                        if self.prices.remove(&price_key).is_some() {
                            changed = true;
                        }
                    } else {
                        let old = self.prices.insert(price_key, size);
                        if old != Some(size) {
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.generation += 1;
        }
        changed
    }

    fn to_price_sizes(&self) -> Vec<PriceSize> {
        let mut items: Vec<_> = self
            .prices
            .iter()
            .map(|(&k, &v)| (f64::from_bits(k), v))
            .collect();

        items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        items
            .into_iter()
            .map(|(price, size)| PriceSize { price, size })
            .collect()
    }
}

/// Runner cache (internal)
struct RunnerCache {
    selection_id: i64,
    handicap: f64,
    status: String,
    adjustment_factor: Option<f64>,
    last_price_traded: Option<f64>,
    total_matched: f64,
    removal_date: Option<String>,
    available_to_back: PriceLadder,
    available_to_lay: PriceLadder,
    traded: TradedLadder,
    sp_near_price: SpValue,
    sp_far_price: SpValue,
    sp_actual_sp: SpValue,
    sp_back_stake_taken: PriceLadder,
    sp_lay_liability_taken: PriceLadder,
    // Generation tracking for object reuse
    generation: u64,
    cached_runner: Option<Py<RunnerBook>>,
    cached_generation: u64,
    // Cached Python list objects for price ladders (reused when unchanged)
    cached_back_list: Option<PyObject>,
    cached_back_gen: u64,
    cached_lay_list: Option<PyObject>,
    cached_lay_gen: u64,
    cached_traded_list: Option<PyObject>,
    cached_traded_gen: u64,
}

impl RunnerCache {
    fn new(selection_id: i64, handicap: f64) -> Self {
        RunnerCache {
            selection_id,
            handicap,
            status: "ACTIVE".to_string(),
            adjustment_factor: None,
            last_price_traded: None,
            total_matched: 0.0,
            removal_date: None,
            available_to_back: PriceLadder::new(true),
            available_to_lay: PriceLadder::new(false),
            traded: TradedLadder::new(),
            sp_near_price: SpValue::None,
            sp_far_price: SpValue::None,
            sp_actual_sp: SpValue::None,
            sp_back_stake_taken: PriceLadder::new(false),
            sp_lay_liability_taken: PriceLadder::new(true),  // descending (matches bflw starting_price_back)
            generation: 0,
            cached_runner: None,
            cached_generation: 0,
            cached_back_list: None,
            cached_back_gen: 0,
            cached_lay_list: None,
            cached_lay_gen: 0,
            cached_traded_list: None,
            cached_traded_gen: 0,
        }
    }

    fn update(&mut self, rc: &Value) -> bool {
        let mut changed = false;

        if let Some(s) = rc.get("status").and_then(|v| v.as_str()) {
            if self.status != s {
                self.status = s.to_string();
                changed = true;
            }
        }
        if let Some(v) = rc.get("adjustmentFactor").and_then(|v| v.as_f64()) {
            if self.adjustment_factor != Some(v) {
                self.adjustment_factor = Some(v);
                changed = true;
            }
        }
        if let Some(v) = rc.get("ltp").and_then(|v| v.as_f64()) {
            if self.last_price_traded != Some(v) {
                self.last_price_traded = Some(v);
                changed = true;
            }
        }
        if let Some(v) = rc.get("tv").and_then(|v| v.as_f64()) {
            if self.total_matched != v {
                self.total_matched = v;
                changed = true;
            }
        }
        if let Some(s) = rc.get("removalDate").and_then(|v| v.as_str()) {
            if self.removal_date.as_deref() != Some(s) {
                self.removal_date = Some(s.to_string());
                changed = true;
            }
        }
        // Counter-intuitive mapping matching betfairlightweight:
        // spb (starting price back) -> lay_liability_taken
        // spl (starting price lay)  -> back_stake_taken
        if let Some(arr) = rc.get("spb").and_then(|v| v.as_array()) {
            if self.sp_lay_liability_taken.update(arr) {
                changed = true;
            }
        }
        if let Some(arr) = rc.get("spl").and_then(|v| v.as_array()) {
            if self.sp_back_stake_taken.update(arr) {
                changed = true;
            }
        }
        // Note: spa (starting price actual) from runner changes is NOT used by
        // betfairlightweight. actualSP comes from definition's bsp field instead.
        if let Some(arr) = rc.get("atb").and_then(|v| v.as_array()) {
            if self.available_to_back.update(arr) {
                changed = true;
            }
        }
        if let Some(arr) = rc.get("atl").and_then(|v| v.as_array()) {
            if self.available_to_lay.update(arr) {
                changed = true;
            }
        }
        if let Some(arr) = rc.get("trd").and_then(|v| v.as_array()) {
            if self.traded.update(arr) {
                changed = true;
            }
        }
        if let Some(v) = rc.get("spn") {
            self.sp_near_price = SpValue::from_json(v);
            changed = true;
        }
        if let Some(v) = rc.get("spf") {
            self.sp_far_price = SpValue::from_json(v);
            changed = true;
        }

        if changed {
            self.generation += 1;
        }
        changed
    }

    fn update_from_definition(&mut self, def: &Value) -> bool {
        let mut changed = false;

        if let Some(s) = def.get("status").and_then(|v| v.as_str()) {
            if self.status != s {
                self.status = s.to_string();
                changed = true;
            }
        }
        // actualSP comes from definition's bsp field (matching betfairlightweight)
        let new_bsp = def.get("bsp")
            .map(|v| SpValue::from_json(v))
            .unwrap_or(SpValue::None);
        self.sp_actual_sp = new_bsp;
        changed = true;

        if let Some(v) = def.get("adjustmentFactor").and_then(|v| v.as_f64()) {
            if self.adjustment_factor != Some(v) {
                self.adjustment_factor = Some(v);
                changed = true;
            }
        }
        if let Some(s) = def.get("removalDate").and_then(|v| v.as_str()) {
            if self.removal_date.as_deref() != Some(s) {
                self.removal_date = Some(s.to_string());
                changed = true;
            }
        }

        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Helper to create a Python list from PriceSize items
    fn create_price_list(py: Python<'_>, items: &[PriceSize]) -> PyObject {
        let list = PyList::new_bound(py, items.iter().map(|ps| ps.clone().into_py(py)));
        list.into()
    }

    /// Get or create cached back list
    fn get_back_list(&mut self, py: Python<'_>) -> PyObject {
        if self.cached_back_gen == self.available_to_back.generation {
            if let Some(ref cached) = self.cached_back_list {
                return cached.clone_ref(py);
            }
        }
        let items = self.available_to_back.to_price_sizes();
        let list = Self::create_price_list(py, &items);
        self.cached_back_list = Some(list.clone_ref(py));
        self.cached_back_gen = self.available_to_back.generation;
        list
    }

    /// Get or create cached lay list
    fn get_lay_list(&mut self, py: Python<'_>) -> PyObject {
        if self.cached_lay_gen == self.available_to_lay.generation {
            if let Some(ref cached) = self.cached_lay_list {
                return cached.clone_ref(py);
            }
        }
        let items = self.available_to_lay.to_price_sizes();
        let list = Self::create_price_list(py, &items);
        self.cached_lay_list = Some(list.clone_ref(py));
        self.cached_lay_gen = self.available_to_lay.generation;
        list
    }

    /// Get or create cached traded list
    fn get_traded_list(&mut self, py: Python<'_>) -> PyObject {
        if self.cached_traded_gen == self.traded.generation {
            if let Some(ref cached) = self.cached_traded_list {
                return cached.clone_ref(py);
            }
        }
        let items = self.traded.to_price_sizes();
        let list = Self::create_price_list(py, &items);
        self.cached_traded_list = Some(list.clone_ref(py));
        self.cached_traded_gen = self.traded.generation;
        list
    }

    /// Get or create the Python RunnerBook object, reusing cached version if unchanged
    fn to_runner(&mut self, py: Python<'_>) -> PyResult<Py<RunnerBook>> {
        // If we have a cached version and generation matches, reuse it
        if let Some(ref cached) = self.cached_runner {
            if self.cached_generation == self.generation {
                return Ok(cached.clone_ref(py));
            }
        }

        let sp = StartingPrices {
            near_price: self.sp_near_price.clone(),
            far_price: self.sp_far_price.clone(),
            actual_sp: self.sp_actual_sp.clone(),
            back_stake_taken: self.sp_back_stake_taken.to_price_sizes(),
            lay_liability_taken: self.sp_lay_liability_taken.to_price_sizes(),
        };

        // Get or create cached price lists
        let back_list = self.get_back_list(py);
        let lay_list = self.get_lay_list(py);
        let traded_list = self.get_traded_list(py);

        let ex = Py::new(py, ExchangePrices {
            cached_back: back_list,
            cached_lay: lay_list,
            cached_traded: traded_list,
        })?;

        // Pre-cache status as Python string (avoids creating new PyString each access)
        let cached_status = self.status.clone().into_py(py);

        let runner = RunnerBook {
            selection_id: self.selection_id,
            handicap: self.handicap,
            cached_status,
            adjustment_factor: self.adjustment_factor,
            last_price_traded: self.last_price_traded,
            total_matched: self.total_matched,
            removal_date: self.removal_date.clone(),
            ex,
            sp,
        };

        let py_runner = Py::new(py, runner)?;
        self.cached_runner = Some(py_runner.clone_ref(py));
        self.cached_generation = self.generation;
        Ok(py_runner)
    }
}

/// Market cache (internal) - the main cache structure
pub struct MarketCache {
    market_id: String,
    publish_time: i64,
    status: String,
    bet_delay: i32,
    bsp_reconciled: bool,
    complete: bool,
    cross_matching: bool,
    inplay: bool,
    number_of_active_runners: i32,
    number_of_runners: i32,
    number_of_winners: i32,
    runners_voidable: bool,
    total_matched: f64,
    version: i64,
    last_match_time: Option<i64>,
    runners: HashMap<(i64, i64), RunnerCache>,
    // Market definition fields
    event_id: Option<String>,
    event_type_id: Option<String>,
    event_name: Option<String>,
    market_type: Option<String>,
    market_time: Option<String>,
    country_code: Option<String>,
    venue: Option<String>,
    race_type: Option<String>,
    name: Option<String>,
    bsp_market: bool,
    persistence_enabled: bool,
    each_way_divisor: Option<f64>,
    timezone: Option<String>,
    turn_in_play_enabled: bool,
    betting_type: Option<String>,
    discount_allowed: bool,
    market_base_rate: f64,
    regulators: Vec<String>,
    open_date: Option<String>,
    suspend_time: Option<String>,
    settled_time: Option<String>,
    // Cached definition runner data (pre-built)
    def_runners: Vec<MarketDefinitionRunner>,
    // Generation tracking for market definition caching
    def_generation: u64,
    cached_definition: Option<Py<MarketDefinition>>,
    cached_def_generation: u64,
}

impl MarketCache {
    pub fn new(market_id: String) -> Self {
        MarketCache {
            market_id,
            publish_time: 0,
            status: "OPEN".to_string(),
            bet_delay: 0,
            bsp_reconciled: false,
            complete: true,
            cross_matching: true,
            inplay: false,
            number_of_active_runners: 0,
            number_of_runners: 0,
            number_of_winners: 1,
            runners_voidable: false,
            total_matched: 0.0,
            version: 0,
            last_match_time: None,
            runners: HashMap::new(),
            event_id: None,
            event_type_id: None,
            event_name: None,
            market_type: None,
            market_time: None,
            country_code: None,
            venue: None,
            race_type: None,
            name: None,
            bsp_market: false,
            persistence_enabled: true,
            each_way_divisor: None,
            timezone: None,
            turn_in_play_enabled: false,
            betting_type: None,
            discount_allowed: false,
            market_base_rate: 0.0,
            regulators: Vec::new(),
            open_date: None,
            suspend_time: None,
            settled_time: None,
            def_runners: Vec::new(),
            def_generation: 0,
            cached_definition: None,
            cached_def_generation: 0,
        }
    }

    pub fn update(&mut self, mc: &Value, publish_time: i64) {
        self.publish_time = publish_time;

        if mc.get("img").and_then(|v| v.as_bool()).unwrap_or(false) {
            self.runners.clear();
        }

        if let Some(def) = mc.get("marketDefinition") {
            self.update_definition(def);
        }

        if let Some(s) = mc.get("status").and_then(|v| v.as_str()) {
            self.status = s.to_string();
        }
        if let Some(v) = mc.get("betDelay").and_then(|v| v.as_i64()) {
            self.bet_delay = v as i32;
        }
        if let Some(v) = mc.get("bspReconciled").and_then(|v| v.as_bool()) {
            self.bsp_reconciled = v;
        }
        if let Some(v) = mc.get("complete").and_then(|v| v.as_bool()) {
            self.complete = v;
        }
        if let Some(v) = mc.get("crossMatching").and_then(|v| v.as_bool()) {
            self.cross_matching = v;
        }
        if let Some(v) = mc.get("inPlay").and_then(|v| v.as_bool()) {
            self.inplay = v;
        }
        if let Some(v) = mc.get("runnersVoidable").and_then(|v| v.as_bool()) {
            self.runners_voidable = v;
        }
        if let Some(v) = mc.get("tv").and_then(|v| v.as_f64()) {
            self.total_matched = v;
        }
        if let Some(v) = mc.get("version").and_then(|v| v.as_i64()) {
            self.version = v;
        }
        if let Some(v) = mc.get("lastMatchTime").and_then(|v| v.as_i64()) {
            self.last_match_time = Some(v);
        }

        if let Some(arr) = mc.get("rc").and_then(|v| v.as_array()) {
            for rc in arr {
                if let Some(id) = rc.get("id").and_then(|v| v.as_i64()) {
                    let hc = rc.get("hc").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let key = (id, (hc * 100.0).round() as i64);
                    let runner = self
                        .runners
                        .entry(key)
                        .or_insert_with(|| RunnerCache::new(id, hc));
                    runner.update(rc);
                }
            }
        }

        self.number_of_runners = self.runners.len() as i32;
        self.number_of_active_runners = self
            .runners
            .values()
            .filter(|r| r.status == "ACTIVE")
            .count() as i32;
    }

    fn update_definition(&mut self, def: &Value) {
        if let Some(s) = def.get("status").and_then(|v| v.as_str()) {
            self.status = s.to_string();
        }
        if let Some(v) = def.get("betDelay").and_then(|v| v.as_i64()) {
            self.bet_delay = v as i32;
        }
        if let Some(v) = def.get("bspReconciled").and_then(|v| v.as_bool()) {
            self.bsp_reconciled = v;
        }
        if let Some(v) = def.get("complete").and_then(|v| v.as_bool()) {
            self.complete = v;
        }
        if let Some(v) = def.get("crossMatching").and_then(|v| v.as_bool()) {
            self.cross_matching = v;
        }
        if let Some(v) = def.get("inPlay").and_then(|v| v.as_bool()) {
            self.inplay = v;
        }
        if let Some(v) = def.get("runnersVoidable").and_then(|v| v.as_bool()) {
            self.runners_voidable = v;
        }
        if let Some(v) = def.get("numberOfWinners").and_then(|v| v.as_i64()) {
            self.number_of_winners = v as i32;
        }

        // Market definition fields
        if let Some(s) = def.get("eventId").and_then(|v| v.as_str()) {
            self.event_id = Some(s.to_string());
        }
        if let Some(s) = def.get("eventTypeId").and_then(|v| v.as_str()) {
            self.event_type_id = Some(s.to_string());
        }
        if let Some(s) = def.get("eventName").and_then(|v| v.as_str()) {
            self.event_name = Some(s.to_string());
        }
        if let Some(s) = def.get("marketType").and_then(|v| v.as_str()) {
            self.market_type = Some(s.to_string());
        }
        if let Some(s) = def.get("marketTime").and_then(|v| v.as_str()) {
            self.market_time = Some(s.to_string());
        }
        if let Some(s) = def.get("countryCode").and_then(|v| v.as_str()) {
            self.country_code = Some(s.to_string());
        }
        if let Some(s) = def.get("venue").and_then(|v| v.as_str()) {
            self.venue = Some(s.to_string());
        }
        if let Some(s) = def.get("raceType").and_then(|v| v.as_str()) {
            self.race_type = Some(s.to_string());
        }
        if let Some(v) = def.get("bspMarket").and_then(|v| v.as_bool()) {
            self.bsp_market = v;
        }
        if let Some(v) = def.get("persistenceEnabled").and_then(|v| v.as_bool()) {
            self.persistence_enabled = v;
        }
        if let Some(v) = def.get("eachWayDivisor").and_then(|v| v.as_f64()) {
            self.each_way_divisor = Some(v);
        }
        if let Some(s) = def.get("timezone").and_then(|v| v.as_str()) {
            self.timezone = Some(s.to_string());
        }
        if let Some(v) = def.get("turnInPlayEnabled").and_then(|v| v.as_bool()) {
            self.turn_in_play_enabled = v;
        }
        if let Some(s) = def.get("bettingType").and_then(|v| v.as_str()) {
            self.betting_type = Some(s.to_string());
        }
        if let Some(s) = def.get("name").and_then(|v| v.as_str()) {
            self.name = Some(s.to_string());
        }
        if let Some(v) = def.get("discountAllowed").and_then(|v| v.as_bool()) {
            self.discount_allowed = v;
        }
        if let Some(v) = def.get("marketBaseRate").and_then(|v| v.as_f64()) {
            self.market_base_rate = v;
        }
        if let Some(arr) = def.get("regulators").and_then(|v| v.as_array()) {
            self.regulators = arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(s) = def.get("openDate").and_then(|v| v.as_str()) {
            self.open_date = Some(s.to_string());
        }
        if let Some(s) = def.get("suspendTime").and_then(|v| v.as_str()) {
            self.suspend_time = Some(s.to_string());
        }
        if let Some(s) = def.get("settledTime").and_then(|v| v.as_str()) {
            self.settled_time = Some(s.to_string());
        }
        if let Some(v) = def.get("version").and_then(|v| v.as_i64()) {
            self.version = v;
        }

        self.def_generation += 1;

        if let Some(arr) = def.get("runners").and_then(|v| v.as_array()) {
            self.def_runners = arr.iter().map(|r| {
                MarketDefinitionRunner {
                    selection_id: r.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                    sort_priority: r.get("sortPriority").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    status: r.get("status").and_then(|v| v.as_str()).unwrap_or("ACTIVE").to_string(),
                    handicap: r.get("hc").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    adjustment_factor: r.get("adjustmentFactor").and_then(|v| v.as_f64()),
                    removal_date: r.get("removalDate").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    name: r.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                    bsp: r.get("bsp").and_then(|v| v.as_f64()),
                }
            }).collect();
            for r in arr {
                if let Some(id) = r.get("id").and_then(|v| v.as_i64()) {
                    let hc = r.get("hc").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let key = (id, (hc * 100.0).round() as i64);
                    let runner = self
                        .runners
                        .entry(key)
                        .or_insert_with(|| RunnerCache::new(id, hc));
                    runner.update_from_definition(r);
                }
            }
        }
    }

    /// Create a MarketBook Python object with runner object reuse
    pub fn to_market_book(&mut self, py: Python<'_>) -> PyResult<Py<MarketBook>> {
        // Get runners sorted by key, reusing cached RunnerBook objects
        let mut keys: Vec<_> = self.runners.keys().cloned().collect();
        keys.sort();

        let mut runners: Vec<Py<RunnerBook>> = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(runner_cache) = self.runners.get_mut(&key) {
                runners.push(runner_cache.to_runner(py)?);
            }
        }

        // Get or create cached MarketDefinition (only rebuilt when definition changes)
        let market_definition = if self.cached_def_generation == self.def_generation {
            self.cached_definition.as_ref().map(|d| d.clone_ref(py))
        } else {
            let cached_runners_list = PyList::new_bound(
                py,
                self.def_runners.iter().map(|r| r.clone().into_py(py)),
            ).into();
            let cached_market_time = parse_datetime_or_none(py, &self.market_time)?;
            let cached_open_date = parse_datetime_or_none(py, &self.open_date)?;
            let cached_suspend_time = parse_datetime_or_none(py, &self.suspend_time)?;
            let cached_settled_time = parse_datetime_or_none(py, &self.settled_time)?;

            let def = Py::new(py, MarketDefinition {
                event_id: self.event_id.clone(),
                event_type_id: self.event_type_id.clone(),
                event_name: self.event_name.clone(),
                market_type: self.market_type.clone(),
                country_code: self.country_code.clone(),
                venue: self.venue.clone(),
                race_type: self.race_type.clone(),
                name: self.name.clone(),
                timezone: self.timezone.clone(),
                betting_type: self.betting_type.clone(),
                regulators: self.regulators.clone(),
                cached_runners: cached_runners_list,
                cached_market_time,
                cached_open_date,
                cached_suspend_time,
                cached_settled_time,
                raw_market_time: self.market_time.clone(),
                raw_open_date: self.open_date.clone(),
                raw_suspend_time: self.suspend_time.clone(),
                bsp_market: self.bsp_market,
                persistence_enabled: self.persistence_enabled,
                each_way_divisor: self.each_way_divisor,
                turn_in_play_enabled: self.turn_in_play_enabled,
                discount_allowed: self.discount_allowed,
                market_base_rate: self.market_base_rate,
                bet_delay: self.bet_delay,
                number_of_winners: self.number_of_winners,
                number_of_active_runners: self.number_of_active_runners,
                bsp_reconciled: self.bsp_reconciled,
                complete: self.complete,
                cross_matching: self.cross_matching,
                in_play: self.inplay,
                runners_voidable: self.runners_voidable,
                status: self.status.clone(),
                version: self.version,
            })?;
            self.cached_definition = Some(def.clone_ref(py));
            self.cached_def_generation = self.def_generation;
            Some(def)
        };

        // Create cached publish_time as a timezone-aware (UTC) datetime
        // (matching current betfairlightweight)
        let timestamp = self.publish_time as f64 / 1000.0;
        let datetime_mod = py.import_bound("datetime")?;
        let datetime_cls = datetime_mod.getattr("datetime")?;
        let utc = datetime_mod.getattr("timezone")?.getattr("utc")?;
        let cached_publish_time: PyObject = datetime_cls
            .call_method1("fromtimestamp", (timestamp, &utc))?
            .into();

        // Create cached runners list
        let cached_runners = PyList::new_bound(py, runners.iter().map(|r| r.clone_ref(py))).into();

        let market_book = MarketBook {
            market_id: self.market_id.clone(),
            publish_time_millis: self.publish_time,
            cached_publish_time,
            cached_runners,
            status: self.status.clone(),
            bet_delay: self.bet_delay,
            bsp_reconciled: self.bsp_reconciled,
            complete: self.complete,
            cross_matching: self.cross_matching,
            inplay: self.inplay,
            number_of_active_runners: self.number_of_active_runners,
            number_of_runners: self.number_of_runners,
            number_of_winners: self.number_of_winners,
            runners_voidable: self.runners_voidable,
            total_matched: self.total_matched,
            version: self.version,
            last_match_time: self.last_match_time,
            market_definition,
            streaming_unique_id: None,
        };

        Py::new(py, market_book)
    }
}
