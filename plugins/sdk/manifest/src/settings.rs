//! Declarative settings pages.
//!
//! A plugin describes its settings as a list of [`Field`]s in `plugin.toml`.
//! The app renders the page with its own widgets (so it follows the app theme),
//! validates input with [`Field::validate`], stores the values on the device,
//! and passes them to every hook in [`crate::abi::HookInput::settings`].
//!
//! ```toml
//! [[settings]]
//! key = "min_age_days"
//! label = "Only screenshots older than"
//! type = "number"
//! min = 0
//! max = 3650
//! default = 30
//! unit = "days"
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A JSON object of setting values, keyed by [`Field::key`].
pub type Values = Map<String, Value>;

/// One choice of a [`FieldKind::Select`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectOption {
    /// Stored value.
    pub value: String,
    /// Label shown to the user.
    pub label: String,
}

/// The widget and value type of a [`Field`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldKind {
    /// On/off switch. Value: boolean.
    Toggle {
        /// Initial value.
        #[serde(default)]
        default: bool,
    },
    /// Text box. Value: string.
    Text {
        /// Initial value.
        #[serde(default)]
        default: String,
        /// Render as a multi-line text area.
        #[serde(default)]
        multiline: bool,
        /// Maximum length in characters (host default 4096).
        #[serde(default)]
        max_length: Option<u32>,
        /// Greyed-out hint shown in an empty box.
        #[serde(default)]
        placeholder: Option<String>,
    },
    /// Number input. Value: number.
    Number {
        /// Initial value.
        #[serde(default)]
        default: f64,
        /// Smallest allowed value.
        #[serde(default)]
        min: Option<f64>,
        /// Largest allowed value.
        #[serde(default)]
        max: Option<f64>,
        /// Only whole numbers.
        #[serde(default)]
        integer: bool,
        /// Unit shown after the box, e.g. "days".
        #[serde(default)]
        unit: Option<String>,
    },
    /// Drop-down. Value: one of the option values.
    Select {
        /// Choices.
        options: Vec<SelectOption>,
        /// Initial value; must be one of `options`.
        default: String,
    },
    /// Album picker. Value: album id (number) or null.
    Album {
        /// Allow choosing more than one album. Value is then an array of ids.
        #[serde(default)]
        multiple: bool,
    },
}

/// One entry on a settings page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// Key in the values object. `[a-z0-9_]`, at most 64 characters.
    pub key: String,
    /// Label shown to the user.
    pub label: String,
    /// Optional help text under the field.
    #[serde(default)]
    pub help: Option<String>,
    /// Widget and value type.
    #[serde(flatten)]
    pub kind: FieldKind,
}

/// Why a setting value was rejected.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SettingsError {
    /// Field definition is invalid.
    #[error("setting `{0}`: {1}")]
    BadField(String, &'static str),
    /// Value does not match the field.
    #[error("setting `{0}`: {1}")]
    BadValue(String, &'static str),
}

impl Field {
    /// The value used when the user has not changed the field.
    pub fn default_value(&self) -> Value {
        match &self.kind {
            FieldKind::Toggle { default } => Value::Bool(*default),
            FieldKind::Text { default, .. } => Value::String(default.clone()),
            FieldKind::Number { default, .. } => serde_json::Number::from_f64(*default)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            FieldKind::Select { default, .. } => Value::String(default.clone()),
            FieldKind::Album { multiple: true } => Value::Array(vec![]),
            FieldKind::Album { multiple: false } => Value::Null,
        }
    }

    /// Check the field definition itself.
    pub fn check_definition(&self) -> Result<(), SettingsError> {
        let bad = |m| Err(SettingsError::BadField(self.key.clone(), m));
        if self.key.is_empty()
            || self.key.len() > 64
            || !self
                .key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return bad("key must be 1-64 characters of a-z, 0-9 and _");
        }
        if self.label.trim().is_empty() {
            return bad("label is empty");
        }
        match &self.kind {
            FieldKind::Number { min, max, .. } => {
                if let (Some(a), Some(b)) = (min, max)
                    && a > b
                {
                    return bad("min is greater than max");
                }
            }
            FieldKind::Select { options, .. } => {
                if options.is_empty() {
                    return bad("select has no options");
                }
            }
            _ => {}
        }
        // The default must itself be a valid value.
        self.validate(&self.default_value())
            .map_err(|_| SettingsError::BadField(self.key.clone(), "default is not a valid value"))
    }

    /// Check a value the user entered.
    pub fn validate(&self, v: &Value) -> Result<(), SettingsError> {
        let bad = |m| Err(SettingsError::BadValue(self.key.clone(), m));
        match (&self.kind, v) {
            (FieldKind::Toggle { .. }, Value::Bool(_)) => Ok(()),
            (FieldKind::Text { max_length, .. }, Value::String(s)) => {
                if s.chars().count() > max_length.unwrap_or(4096) as usize {
                    bad("text is too long")
                } else {
                    Ok(())
                }
            }
            (
                FieldKind::Number {
                    min, max, integer, ..
                },
                Value::Number(n),
            ) => {
                let Some(x) = n.as_f64() else {
                    return bad("not a number");
                };
                if min.is_some_and(|m| x < m) || max.is_some_and(|m| x > m) {
                    bad("number is out of range")
                } else if *integer && x.fract() != 0.0 {
                    bad("must be a whole number")
                } else {
                    Ok(())
                }
            }
            (FieldKind::Select { options, .. }, Value::String(s)) => {
                if options.iter().any(|o| &o.value == s) {
                    Ok(())
                } else {
                    bad("not one of the options")
                }
            }
            (FieldKind::Album { multiple: false }, Value::Null) => Ok(()),
            (FieldKind::Album { multiple: false }, Value::Number(n)) if n.is_i64() => Ok(()),
            (FieldKind::Album { multiple: true }, Value::Array(a))
                if a.iter().all(|x| x.is_i64()) =>
            {
                Ok(())
            }
            _ => bad("wrong type"),
        }
    }
}

/// Merge `stored` values over the defaults of `fields`, dropping unknown keys
/// and invalid values (which fall back to the default). This is what the host
/// passes to hooks, so a plugin always sees a complete, valid object.
pub fn effective_values(fields: &[Field], stored: &Values) -> Values {
    fields
        .iter()
        .map(|f| {
            let v = stored
                .get(&f.key)
                .filter(|v| f.validate(v).is_ok())
                .cloned()
                .unwrap_or_else(|| f.default_value());
            (f.key.clone(), v)
        })
        .collect()
}

/// Validate a complete update from the settings page. Unknown keys are errors.
pub fn validate_values(fields: &[Field], values: &Values) -> Result<(), SettingsError> {
    for (k, v) in values {
        let f = fields
            .iter()
            .find(|f| &f.key == k)
            .ok_or_else(|| SettingsError::BadValue(k.clone(), "unknown setting"))?;
        f.validate(v)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields() -> Vec<Field> {
        toml::from_str::<Wrap>(
            r#"
            [[settings]]
            key = "days"
            label = "Days"
            type = "number"
            min = 0
            max = 10
            integer = true
            default = 3

            [[settings]]
            key = "mode"
            label = "Mode"
            type = "select"
            options = [{ value = "a", label = "A" }, { value = "b", label = "B" }]
            default = "a"

            [[settings]]
            key = "on"
            label = "On"
            type = "toggle"
            "#,
        )
        .unwrap()
        .settings
    }

    #[derive(Deserialize)]
    struct Wrap {
        settings: Vec<Field>,
    }

    #[test]
    fn definitions_and_defaults() {
        let f = fields();
        for x in &f {
            x.check_definition().unwrap();
        }
        let v = effective_values(&f, &Values::new());
        assert_eq!(json!(v), json!({"days": 3.0, "mode": "a", "on": false}));
    }

    #[test]
    fn validation() {
        let f = fields();
        assert!(f[0].validate(&json!(11)).is_err());
        assert!(f[0].validate(&json!(2.5)).is_err());
        assert!(f[0].validate(&json!(10)).is_ok());
        assert!(f[1].validate(&json!("c")).is_err());
        assert!(f[2].validate(&json!("yes")).is_err());
        let mut bad = Values::new();
        bad.insert("nope".into(), json!(1));
        assert!(validate_values(&f, &bad).is_err());
        // An invalid stored value falls back to the default.
        let mut stored = Values::new();
        stored.insert("days".into(), json!(99));
        assert_eq!(effective_values(&f, &stored)["days"], json!(3.0));
    }
}
