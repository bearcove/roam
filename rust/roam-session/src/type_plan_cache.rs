use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

use facet_core::Shape;
use facet_reflect::TypePlanCore;

static TYPE_PLAN_CACHE: LazyLock<Mutex<HashMap<usize, Arc<TypePlanCore>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static DISABLE_CACHE: LazyLock<bool> = LazyLock::new(|| {
    let Ok(value) = std::env::var("ROAM_DISABLE_TYPE_PLAN_CACHE") else {
        return false;
    };

    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
});

#[allow(unsafe_code)]
pub(crate) fn get_type_plan(shape: &'static Shape) -> Result<Arc<TypePlanCore>, String> {
    if *DISABLE_CACHE {
        // SAFETY: `shape` comes from a `Facet` implementation.
        return unsafe { TypePlanCore::from_shape(shape) }.map_err(|e| e.to_string());
    }

    let key = shape as *const Shape as usize;

    if let Some(plan) = TYPE_PLAN_CACHE.lock().unwrap().get(&key).cloned() {
        return Ok(plan);
    }

    // SAFETY: `shape` comes from a `Facet` implementation.
    let plan = unsafe { TypePlanCore::from_shape(shape) }.map_err(|e| e.to_string())?;

    let mut cache = TYPE_PLAN_CACHE.lock().unwrap();
    Ok(cache.entry(key).or_insert_with(|| plan.clone()).clone())
}
