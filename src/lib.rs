//! tickrush: High-performance Betfair price stream reader

pub(crate) mod market;
pub(crate) mod parse;

use crate::market::MarketCache;
use crate::parse::{parse_messages, ParsedMessage};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use std::path::Path;

/// Iterator that yields MarketBook objects with object reuse
#[pyclass]
pub struct PricesIterator {
    messages: Vec<ParsedMessage>,
    index: usize,
    cache: Option<MarketCache>,
}

#[pymethods]
impl PricesIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Option<PyObject>> {
        if slf.index >= slf.messages.len() {
            return Ok(None);
        }

        let publish_time = slf.messages[slf.index].publish_time;
        let market_changes = slf.messages[slf.index].market_changes.clone();
        slf.index += 1;

        for mc in &market_changes {
            let market_id = mc
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if slf.cache.is_none() && !market_id.is_empty() {
                slf.cache = Some(MarketCache::new(market_id));
            }

            if let Some(ref mut c) = slf.cache {
                c.update(mc, publish_time);
            }
        }

        if let Some(ref mut c) = slf.cache {
            let market_book = c.to_market_book(py)?;
            return Ok(Some(market_book.into_py(py)));
        }

        Ok(None)
    }

    fn __len__(&self) -> usize {
        self.messages.len()
    }
}

#[pyfunction]
#[pyo3(signature = (path,))]
fn iter_prices_file(path: &str) -> PyResult<PricesIterator> {
    let input_path = Path::new(path);
    let messages = parse_messages(input_path).map_err(|e| PyValueError::new_err(e))?;

    Ok(PricesIterator {
        messages,
        index: 0,
        cache: None,
    })
}

#[pymodule]
#[pyo3(name = "tickrush")]
pub fn tickrush(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(iter_prices_file, m)?)?;
    m.add_class::<PricesIterator>()?;
    m.add_class::<market::MarketBook>()?;
    m.add_class::<market::RunnerBook>()?;
    m.add_class::<market::ExchangePrices>()?;
    m.add_class::<market::StartingPrices>()?;
    m.add_class::<market::PriceSize>()?;
    m.add_class::<market::MarketDefinition>()?;
    m.add_class::<market::MarketDefinitionRunner>()?;
    Ok(())
}
