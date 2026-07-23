// Copyright (C) 2026 Rusty Fleet contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::{ContractViolation, ValidateContract, finish, require_nonempty};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryField {
    DeviceId,
    DisplayName,
    Model,
    Tag,
    Freshness,
    BatteryPercent,
    ForegroundApp,
    KioskState,
    Capability,
    ConditionFamily,
    ConditionState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    Equals,
    NotEquals,
    Contains,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    Exists,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QueryValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
    TextList(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryExpression {
    Predicate {
        field: QueryField,
        comparison: Comparison,
        value: Option<QueryValue>,
        qualifier: Option<String>,
    },
    And {
        expressions: Vec<QueryExpression>,
    },
    Or {
        expressions: Vec<QueryExpression>,
    },
    Not {
        expression: Box<QueryExpression>,
    },
}

impl QueryExpression {
    fn validate_into(&self, path: &str, failures: &mut Vec<ContractViolation>) {
        match self {
            Self::Predicate {
                field,
                comparison,
                value,
                qualifier,
            } => {
                if *comparison == Comparison::Exists && value.is_some() {
                    failures.push(ContractViolation::new(
                        "unexpected_query_value",
                        path,
                        "exists predicates must not carry a value",
                    ));
                }
                if *comparison != Comparison::Exists && value.is_none() {
                    failures.push(ContractViolation::new(
                        "missing_query_value",
                        path,
                        "non-exists predicates require a value",
                    ));
                }
                if qualifier
                    .as_ref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    failures.push(ContractViolation::new(
                        "empty_qualifier",
                        path,
                        "query qualifier must not be empty",
                    ));
                }
                if matches!(field, QueryField::Tag | QueryField::Capability) && qualifier.is_none()
                {
                    failures.push(ContractViolation::new(
                        "missing_qualifier",
                        path,
                        "tag and capability predicates require a qualifier",
                    ));
                }
                if !matches!(field, QueryField::Tag | QueryField::Capability) && qualifier.is_some()
                {
                    failures.push(ContractViolation::new(
                        "unexpected_qualifier",
                        path,
                        "this query field does not accept a qualifier",
                    ));
                }
                if !predicate_value_is_compatible(*field, *comparison, value.as_ref()) {
                    failures.push(ContractViolation::new(
                        "incompatible_query_value",
                        path,
                        "query value type or comparison is incompatible with the field",
                    ));
                }
            }
            Self::And { expressions } | Self::Or { expressions } => {
                if expressions.len() < 2 {
                    failures.push(ContractViolation::new(
                        "insufficient_query_terms",
                        path,
                        "and/or expressions require at least two terms",
                    ));
                }
                for (index, expression) in expressions.iter().enumerate() {
                    expression.validate_into(&format!("{path}.expressions[{index}]"), failures);
                }
            }
            Self::Not { expression } => {
                expression.validate_into(&format!("{path}.expression"), failures)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortKey {
    pub field: QueryField,
    pub direction: SortDirection,
    pub qualifier: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FleetQuery {
    pub schema: String,
    pub query_id: String,
    pub expression: Option<QueryExpression>,
    #[serde(default)]
    pub sort: Vec<SortKey>,
    pub offset: usize,
    pub limit: usize,
}

impl FleetQuery {
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

impl ValidateContract for FleetQuery {
    fn validate(&self) -> Result<(), Vec<ContractViolation>> {
        let mut failures = Vec::new();
        if self.schema != "rusty.fleet.query.v1" {
            failures.push(ContractViolation::new(
                "wrong_schema",
                "schema",
                "expected rusty.fleet.query.v1",
            ));
        }
        require_nonempty(&mut failures, &self.query_id, "query_id");
        if self.limit == 0 || self.limit > 10_000 {
            failures.push(ContractViolation::new(
                "invalid_window",
                "limit",
                "query limit must be between 1 and 10000",
            ));
        }
        if let Some(expression) = &self.expression {
            expression.validate_into("expression", &mut failures);
        }
        for (index, key) in self.sort.iter().enumerate() {
            if key
                .qualifier
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
            {
                failures.push(ContractViolation::new(
                    "empty_qualifier",
                    &format!("sort[{index}].qualifier"),
                    "sort qualifier must not be empty",
                ));
            }
            if key.field == QueryField::Tag && key.qualifier.is_none() {
                failures.push(ContractViolation::new(
                    "missing_qualifier",
                    &format!("sort[{index}].qualifier"),
                    "tag sorting requires a qualifier",
                ));
            }
            if key.field != QueryField::Tag && key.qualifier.is_some() {
                failures.push(ContractViolation::new(
                    "unexpected_qualifier",
                    &format!("sort[{index}].qualifier"),
                    "only tag sorting accepts a qualifier",
                ));
            }
        }
        finish(failures)
    }
}

fn predicate_value_is_compatible(
    field: QueryField,
    comparison: Comparison,
    value: Option<&QueryValue>,
) -> bool {
    if comparison == Comparison::Exists {
        return value.is_none();
    }
    match field {
        QueryField::BatteryPercent => {
            matches!(value, Some(QueryValue::Integer(_)))
                && matches!(
                    comparison,
                    Comparison::Equals
                        | Comparison::NotEquals
                        | Comparison::LessThan
                        | Comparison::LessThanOrEqual
                        | Comparison::GreaterThan
                        | Comparison::GreaterThanOrEqual
                )
        }
        QueryField::Capability => {
            matches!(value, Some(QueryValue::Boolean(_)))
                && matches!(comparison, Comparison::Equals | Comparison::NotEquals)
        }
        QueryField::DeviceId
        | QueryField::DisplayName
        | QueryField::Model
        | QueryField::Tag
        | QueryField::Freshness
        | QueryField::ForegroundApp
        | QueryField::KioskState
        | QueryField::ConditionFamily
        | QueryField::ConditionState => {
            matches!(value, Some(QueryValue::Text(_) | QueryValue::TextList(_)))
                && matches!(
                    comparison,
                    Comparison::Equals | Comparison::NotEquals | Comparison::Contains
                )
        }
    }
}
