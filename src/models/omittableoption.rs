use std::cmp::Ordering;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A value that can be correctly deserialized from a JSON object field that is either:
/// - omitted (field omitted)
/// - null (explicit null)
/// - a valid value of type `T`
///
/// This is useful when you want to distinguish between a field that is omitted and a field that
/// is explicitly set to null.
///
/// ## Note:
///
/// You must annotate the field with the following to make this work correctly, due to serde limitations:
///
/// ```rust, ignore
/// #[serde(default)]
/// #[serde(skip_serializing_if = "OmittableOption::is_omitted")]
/// some_field: OmittableOption<T>
/// ```
///
#[derive(Debug, Default)]
pub enum OmittableOption<T> {
    Some(T),
    None,
    #[default]
    Omitted,
}

impl<T> OmittableOption<T> {
    /// Returns `true` if the value is `Some`.
    pub const fn is_some(&self) -> bool {
        matches!(self, Self::Some(_))
    }

    /// Returns `true` if the value is `None`.
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns `true` if the value is `Omitted`.
    pub const fn is_omitted(&self) -> bool {
        matches!(self, Self::Omitted)
    }

    /// Map the inner value to a new value using the provided function.
    ///
    /// If the value is `Some`, the function is applied to the inner value and the result is
    /// wrapped in a new `Some`. If the value is `None` or `Omitted`, the function is not applied
    /// and the value is returned as is.
    ///
    /// # Parameters
    ///
    /// - `f` - The function to apply to the inner value.
    ///
    /// # Returns
    ///
    /// A new `OmittableOption` with the result of applying the function to the inner value, or
    /// the original value if it was `None` or `Omitted`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use chat_backend::models::omittableoption::OmittableOption;
    ///
    /// let value = OmittableOption::Some(42);
    ///
    /// let new_value = value.map(|v| v == 10);
    ///
    /// assert_eq!(new_value, OmittableOption::Some(false));
    ///
    /// let value = OmittableOption::<i32>::None;
    ///
    /// let new_value = value.map(|v| v == 10);
    ///
    /// assert_eq!(new_value, OmittableOption::None);
    /// ```
    #[must_use]
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> OmittableOption<U> {
        match self {
            Self::Some(value) => OmittableOption::Some(f(value)),
            Self::None => OmittableOption::None,
            Self::Omitted => OmittableOption::Omitted,
        }
    }

    /// Map the inner value to a new value using the provided function, or return the provided
    /// default value if the value is `None` or `Omitted`.
    ///
    /// # Parameters
    ///
    /// - `default` - The default value to return if the value is `None` or `Omitted`.
    /// - `f` - The function to apply to the inner value.
    ///
    /// # Returns
    ///
    /// The result of applying the function to the inner value if the value is `Some`, otherwise the
    /// provided default value.
    #[must_use]
    pub fn map_or<U>(self, default: U, f: impl FnOnce(T) -> U) -> U {
        match self {
            Self::Some(value) => f(value),
            Self::None | Self::Omitted => default,
        }
    }

    /// Unwrap the value, panicking if the value is `None` or `Omitted`.
    ///
    /// # Returns
    ///
    /// The inner value.
    ///
    /// # Panics
    ///
    /// If the value is `None` or `Omitted`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use chat_backend::models::omittableoption::OmittableOption;
    ///
    /// let value = OmittableOption::Some(42);
    ///
    /// let unwrapped = value.unwrap();
    ///
    /// assert_eq!(unwrapped, 42);
    /// ```
    #[must_use]
    pub fn unwrap(self) -> T {
        match self {
            Self::Some(value) => value,
            Self::None => panic!("Called `unwrap` on a `None` value"),
            Self::Omitted => panic!("Called `unwrap` on an `Omitted` value"),
        }
    }

    /// Unwrap the value, panicking with the provided message if the value is `None` or `Omitted`.
    ///
    /// # Parameters
    ///
    /// - `msg` - The message to include in the panic message.
    ///
    /// # Returns
    ///
    /// The inner value.
    ///
    /// # Panics
    ///
    /// If the value is `None` or `Omitted`.
    #[must_use]
    pub fn expect(self, msg: &str) -> T {
        match self {
            Self::Some(value) => value,
            Self::None => panic!("{}", msg),
            Self::Omitted => panic!("{}", msg),
        }
    }

    /// Unwrap the value, returning the provided default value if the value is `None` or `Omitted`.
    ///
    /// # Parameters
    ///
    /// - `default` - The default value to return if the value is `None` or `Omitted`.
    ///
    /// # Returns
    ///
    /// The inner value if the value is `Some`, otherwise the provided default value.
    #[must_use]
    pub fn unwrap_or(self, default: T) -> T {
        match self {
            Self::Some(value) => value,
            Self::None | Self::Omitted => default,
        }
    }

    /// Unwrap the value, returning the result of the provided function if the value is `None` or
    /// `Omitted`.
    ///
    /// # Parameters
    ///
    /// - `f` - The function to call if the value is `None` or `Omitted`.
    ///
    /// # Returns
    ///
    /// The inner value if the value is `Some`, otherwise the result of calling the provided
    /// function.
    #[must_use]
    pub fn unwrap_or_else(self, f: impl FnOnce() -> T) -> T {
        match self {
            Self::Some(value) => value,
            Self::None | Self::Omitted => f(),
        }
    }
}

impl<T: Default> OmittableOption<T> {
    /// Unwrap the value, returning the default value if the value is `None` or `Omitted`.
    ///
    /// # Returns
    ///
    /// The inner value if the value is `Some`, otherwise the default value of `T`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use chat_backend::models::omittableoption::OmittableOption;
    ///
    /// let value = OmittableOption::Some(42);
    ///
    /// let unwrapped = value.unwrap_or_default();
    ///
    /// assert_eq!(unwrapped, 42);
    ///
    /// let value = OmittableOption::<i32>::None;
    ///
    /// let unwrapped = value.unwrap_or_default();
    ///
    /// assert_eq!(unwrapped, 0);
    ///
    /// let value = OmittableOption::<i32>::Omitted;
    ///
    /// let unwrapped = value.unwrap_or_default();
    ///
    /// assert_eq!(unwrapped, 0);
    /// ```
    #[must_use]
    pub fn unwrap_or_default(self) -> T {
        match self {
            Self::Some(value) => value,
            Self::None | Self::Omitted => Default::default(),
        }
    }
}

impl<T, E> OmittableOption<Result<T, E>> {
    /// Swap the inner `Result` with the outer `OmittableOption`.
    ///
    /// # Returns
    ///
    /// - The transposed value.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use chat_backend::models::omittableoption::OmittableOption;
    ///
    /// let value: OmittableOption<Result<i32, ()>> = OmittableOption::Some(Ok(42));
    ///
    /// let transposed = value.transpose();
    ///
    /// assert_eq!(transposed, Ok(OmittableOption::Some(42)));
    /// ```
    #[expect(clippy::missing_errors_doc)]
    pub fn transpose(self) -> Result<OmittableOption<T>, E> {
        match self {
            Self::Some(Ok(value)) => Ok(OmittableOption::Some(value)),
            Self::Some(Err(err)) => Err(err),
            Self::None => Ok(OmittableOption::None),
            Self::Omitted => Ok(OmittableOption::Omitted),
        }
    }
}

#[expect(clippy::expl_impl_clone_on_copy)]
impl<T: Clone> Clone for OmittableOption<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Some(value) => Self::Some(value.clone()),
            Self::None => Self::None,
            Self::Omitted => Self::Omitted,
        }
    }
}

impl<T: Copy> Copy for OmittableOption<T> {}

impl<T: PartialEq> PartialEq for OmittableOption<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Some(value), Self::Some(other_value)) => value == other_value,
            (Self::None, Self::None) | (Self::Omitted, Self::Omitted) => true,
            _ => false,
        }
    }
}

impl<T: Eq> Eq for OmittableOption<T> {}

impl<T: PartialOrd> PartialOrd for OmittableOption<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Some(value), Self::Some(other_value)) => value.partial_cmp(other_value),
            (Self::None, Self::None) | (Self::Omitted, Self::Omitted) => Some(Ordering::Equal),
            (Self::Some(_), _) | (Self::None, Self::Omitted) => Some(Ordering::Greater),
            (_, Self::Some(_)) | (Self::Omitted, Self::None) => Some(Ordering::Less),
        }
    }
}

impl<T: Ord> Ord for OmittableOption<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Some(value), Self::Some(other_value)) => value.cmp(other_value),
            (Self::None, Self::None) | (Self::Omitted, Self::Omitted) => Ordering::Equal,
            (Self::Some(_), _) | (Self::None, Self::Omitted) => Ordering::Greater,
            (_, Self::Some(_)) | (Self::Omitted, Self::None) => Ordering::Less,
        }
    }
}

impl<T> From<Option<T>> for OmittableOption<T> {
    fn from(option: Option<T>) -> Self {
        option.map_or_else(|| Self::None, |value| Self::Some(value))
    }
}

/// A sentinel value indicating that the value was omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OmittedValue;

impl<T> TryFrom<OmittableOption<T>> for Option<T> {
    type Error = OmittedValue;

    fn try_from(value: OmittableOption<T>) -> Result<Self, Self::Error> {
        match value {
            OmittableOption::Some(value) => Ok(Some(value)),
            OmittableOption::None => Ok(None),
            OmittableOption::Omitted => Err(OmittedValue),
        }
    }
}

impl<T> Serialize for OmittableOption<T>
where
    T: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Omitted => serializer.serialize_unit(),
            Self::None => serializer.serialize_none(),
            Self::Some(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T> Deserialize<'de> for OmittableOption<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::deserialize(deserializer).map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::TryFrom;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct TestWrapper {
        #[serde(default)]
        value: OmittableOption<i32>,
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct MoreComplexTestWrapper {
        #[serde(default)]
        value: OmittableOption<i32>,
        #[serde(default)]
        other_value: OmittableOption<String>,
    }

    #[test]
    fn test_deserialize_omitted() {
        let json_str = "{}";
        let obj: TestWrapper = serde_json::from_str(json_str).expect("Deserialization failed");
        assert_eq!(obj.value, OmittableOption::Omitted);
    }

    #[test]
    fn test_deserialize_null() {
        let json_str = r#"{"value": null}"#;
        let obj: TestWrapper = serde_json::from_str(json_str).expect("Deserialization failed");
        assert_eq!(obj.value, OmittableOption::None);
    }

    #[test]
    fn test_deserialize_some() {
        let json_str = r#"{"value": 42}"#;
        let obj: TestWrapper = serde_json::from_str(json_str).expect("Deserialization failed");
        assert_eq!(obj.value, OmittableOption::Some(42));
    }

    #[test]
    fn test_complex_deserialize_omitted() {
        let json_str = r#"{"value": 42}"#;
        let obj: MoreComplexTestWrapper = serde_json::from_str(json_str).expect("Deserialization failed");
        assert_eq!(obj.value, OmittableOption::Some(42));
        assert_eq!(obj.other_value, OmittableOption::Omitted);
    }

    #[test]
    fn test_complex_deserialize_null() {
        let json_str = r#"{"value": 42, "other_value": null}"#;
        let obj: MoreComplexTestWrapper = serde_json::from_str(json_str).expect("Deserialization failed");
        assert_eq!(obj.value, OmittableOption::Some(42));
        assert_eq!(obj.other_value, OmittableOption::None);
    }

    #[test]
    fn test_complex_deserialize_some() {
        let json_str = r#"{"value": 42, "other_value": "Hello, world!"}"#;
        let obj: MoreComplexTestWrapper = serde_json::from_str(json_str).expect("Deserialization failed");
        assert_eq!(obj.value, OmittableOption::Some(42));
        assert_eq!(obj.other_value, OmittableOption::Some(String::from("Hello, world!")));
    }

    #[test]
    fn test_variant_checks() {
        let some = OmittableOption::Some(10);
        let none: OmittableOption<i32> = OmittableOption::None;
        let omitted: OmittableOption<i32> = OmittableOption::Omitted;

        assert!(some.is_some());
        assert!(!some.is_none());
        assert!(!some.is_omitted());

        assert!(none.is_none());
        assert!(!none.is_some());
        assert!(!none.is_omitted());

        assert!(omitted.is_omitted());
        assert!(!omitted.is_some());
        assert!(!omitted.is_none());
    }

    #[test]
    fn test_map_function() {
        let some = OmittableOption::Some(10);
        let mapped_some = some.map(|x| x * 2);
        assert_eq!(mapped_some, OmittableOption::Some(20));

        let none: OmittableOption<i32> = OmittableOption::None;
        let mapped_none = none.map(|x| x * 2);
        assert_eq!(mapped_none, OmittableOption::None);

        let omitted: OmittableOption<i32> = OmittableOption::Omitted;
        let mapped_omitted = omitted.map(|x| x * 2);
        assert_eq!(mapped_omitted, OmittableOption::Omitted);
    }

    #[test]
    fn test_transpose_result_ok() {
        let opt: OmittableOption<Result<i32, &str>> = OmittableOption::Some(Ok(5));
        let transposed = opt.transpose();
        assert_eq!(transposed, Ok(OmittableOption::Some(5)));
    }

    #[test]
    fn test_transpose_result_err() {
        let opt: OmittableOption<Result<i32, &str>> = OmittableOption::Some(Err("error"));
        let transposed = opt.transpose();
        assert_eq!(transposed, Err("error"));
    }

    #[test]
    fn test_from_option_conversion() {
        let some: Option<i32> = Some(12);
        let opt = OmittableOption::from(some);
        assert_eq!(opt, OmittableOption::Some(12));

        let none: Option<i32> = None;
        let opt_none = OmittableOption::from(none);
        assert_eq!(opt_none, OmittableOption::None);
    }

    #[test]
    fn test_try_from_conversion() {
        let some = OmittableOption::Some(7);
        let converted = Option::try_from(some).expect("Conversion should succeed");
        assert_eq!(converted, Some(7));

        let none: OmittableOption<i32> = OmittableOption::None;
        let converted_none: Option<i32> = Option::try_from(none).expect("Conversion should succeed");
        assert_eq!(converted_none, None);

        let omitted: OmittableOption<i32> = OmittableOption::<i32>::Omitted;
        let result: Result<Option<i32>, OmittedValue> = Option::try_from(omitted);
        assert!(matches!(result, Err(OmittedValue)));
    }

    #[test]
    fn test_clone_method() {
        let original = OmittableOption::Some(String::from("Not copy"));
        let clone = original.clone();
        assert_eq!(original, clone);

        let omitted: OmittableOption<String> = OmittableOption::Omitted;
        let clone_omitted = omitted.clone();
        assert_eq!(omitted, clone_omitted);
    }
}
