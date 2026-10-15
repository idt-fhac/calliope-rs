//! Helper functions (`calliope.backend.helper_functions`): their signatures, where they may
//! be used, and (in [`array`]) their semantics on [`calliope_array::LArray`].

pub mod array;

use crate::ast::Lang;

/// The built-in helper functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Helper {
    /// `any(array, over=...)` (where).
    Any,
    /// `sum(array, over=...)`.
    Sum,
    /// `reduce_carrier_dim(array, flow_direction)`.
    ReduceCarrierDim,
    /// `select_from_lookup_arrays(array, dim=lookup, ...)`.
    SelectFromLookupArrays,
    /// `get_val_at_index(dim=index)`.
    GetValAtIndex,
    /// `roll(array, dim=shift, ...)`.
    Roll,
    /// `where(array, condition)`.
    Where,
    /// `map_dim(dim, mapper)`.
    MapDim,
    /// `group_sum(array, groupby, group_dim)`.
    GroupSum,
    /// `group_datetime(array, over, group)`.
    GroupDatetime,
    /// `sum_next_n(array, over, N)`.
    SumNextN,
}

/// How a helper takes its arguments (its Python signature).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signature {
    /// Fixed parameters: positional-or-keyword, then keyword-only.
    Fixed {
        /// Positional-or-keyword parameter names.
        positional: &'static [&'static str],
        /// Keyword-only parameter names.
        keyword_only: &'static [&'static str],
    },
    /// Positional parameters, then `**kwargs` keyed by dimension.
    DimKwargs {
        /// Positional-or-keyword parameter names.
        positional: &'static [&'static str],
    },
}

impl Helper {
    /// All helpers.
    pub const ALL: [Helper; 11] = [
        Helper::Any,
        Helper::Sum,
        Helper::ReduceCarrierDim,
        Helper::SelectFromLookupArrays,
        Helper::GetValAtIndex,
        Helper::Roll,
        Helper::Where,
        Helper::MapDim,
        Helper::GroupSum,
        Helper::GroupDatetime,
        Helper::SumNextN,
    ];

    /// The name used in math strings (`NAME`).
    pub fn name(self) -> &'static str {
        match self {
            Helper::Any => "any",
            Helper::Sum => "sum",
            Helper::ReduceCarrierDim => "reduce_carrier_dim",
            Helper::SelectFromLookupArrays => "select_from_lookup_arrays",
            Helper::GetValAtIndex => "get_val_at_index",
            Helper::Roll => "roll",
            Helper::Where => "where",
            Helper::MapDim => "map_dim",
            Helper::GroupSum => "group_sum",
            Helper::GroupDatetime => "group_datetime",
            Helper::SumNextN => "sum_next_n",
        }
    }

    /// The helper called `name` in strings of language `lang`, if it is allowed there
    /// (`ALLOWED_IN`, the per-language registry).
    pub fn lookup(name: &str, lang: Lang) -> Option<Helper> {
        Helper::ALL
            .into_iter()
            .find(|h| h.name() == name && h.allowed_in(lang))
    }

    /// `ALLOWED_IN`.
    pub fn allowed_in(self, lang: Lang) -> bool {
        match self {
            Helper::Any => lang == Lang::Where,
            Helper::Sum | Helper::GetValAtIndex | Helper::MapDim => true,
            _ => lang == Lang::Expression,
        }
    }

    /// `ignore_where`: arguments are evaluated without the component's `where` mask.
    pub fn ignore_where(self) -> bool {
        matches!(
            self,
            Helper::SelectFromLookupArrays | Helper::Roll | Helper::Where | Helper::MapDim
        )
    }

    /// The Python signature.
    pub fn signature(self) -> Signature {
        use Signature::{DimKwargs, Fixed};
        match self {
            Helper::Any | Helper::Sum => Fixed {
                positional: &["array"],
                keyword_only: &["over"],
            },
            Helper::ReduceCarrierDim => Fixed {
                positional: &["array", "flow_direction"],
                keyword_only: &[],
            },
            Helper::SelectFromLookupArrays | Helper::Roll => DimKwargs {
                positional: &["array"],
            },
            Helper::GetValAtIndex => DimKwargs { positional: &[] },
            Helper::Where => Fixed {
                positional: &["array", "condition"],
                keyword_only: &[],
            },
            Helper::MapDim => Fixed {
                positional: &["dim", "mapper"],
                keyword_only: &[],
            },
            Helper::GroupSum => Fixed {
                positional: &["array", "groupby", "group_dim"],
                keyword_only: &[],
            },
            Helper::GroupDatetime => Fixed {
                positional: &["array", "over", "group"],
                keyword_only: &[],
            },
            Helper::SumNextN => Fixed {
                positional: &["array", "over", "N"],
                keyword_only: &[],
            },
        }
    }
}

/// Arguments bound to a helper's parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound<T> {
    /// Named parameters, in signature order.
    pub params: Vec<(&'static str, T)>,
    /// `**kwargs` keyed by dimension, in call order.
    pub dim_kwargs: Vec<(String, T)>,
}

impl<T> Bound<T> {
    /// The argument bound to a parameter.
    pub fn get(&self, name: &str) -> Option<&T> {
        self.params.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
}

/// Binds call arguments to a helper's signature as Python would, or explains the
/// `TypeError` Python would raise.
pub fn bind<T>(helper: Helper, args: Vec<T>, kwargs: Vec<(String, T)>) -> Result<Bound<T>, String> {
    let name = helper.name();
    let (positional, keyword_only, var_kwargs) = match helper.signature() {
        Signature::Fixed {
            positional,
            keyword_only,
        } => (positional, keyword_only, false),
        Signature::DimKwargs { positional } => (positional, &[][..], true),
    };
    if args.len() > positional.len() {
        return Err(format!(
            "`{name}` takes {} positional argument(s) but {} were given",
            positional.len(),
            args.len()
        ));
    }
    let mut params: Vec<(&'static str, Option<T>)> = positional
        .iter()
        .chain(keyword_only)
        .map(|p| (*p, None))
        .collect();
    for (slot, a) in params.iter_mut().zip(args) {
        slot.1 = Some(a);
    }
    let mut dim_kwargs = Vec::new();
    for (k, v) in kwargs {
        if let Some(slot) = params.iter_mut().find(|(p, _)| *p == k) {
            if slot.1.is_some() {
                return Err(format!("`{name}` got multiple values for argument `{k}`"));
            }
            slot.1 = Some(v);
        } else if var_kwargs {
            dim_kwargs.push((k, v));
        } else {
            return Err(format!("`{name}` got an unexpected keyword argument `{k}`"));
        }
    }
    let missing: Vec<&str> = params
        .iter()
        .filter(|(_, v)| v.is_none())
        .map(|(p, _)| *p)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "`{name}` is missing required argument(s): {}",
            missing.join(", ")
        ));
    }
    if helper == Helper::GetValAtIndex && dim_kwargs.len() != 1 {
        return Err("Supply one (and only one) dimension:index mapping".to_owned());
    }
    Ok(Bound {
        params: params
            .into_iter()
            .map(|(p, v)| (p, v.expect("checked")))
            .collect(),
        dim_kwargs,
    })
}

/// The pandas datetime accessor attributes `group_datetime` can group by.
pub const DATETIME_GROUPS: [&str; 15] = [
    "date",
    "time",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "day_of_week",
    "weekday",
    "dayofyear",
    "day_of_year",
    "quarter",
    "season",
];
