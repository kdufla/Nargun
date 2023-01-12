#[macro_export]
macro_rules! unsigned_ceil_div {
    ($numerator:expr, $denominator:expr) => {{
        1 + (($numerator - 1) / $denominator)
    }};
}

#[macro_export]
macro_rules! ok_or_missing_field {
    ($field:expr) => {
        $field.ok_or_else(|| bendy::decoding::Error::missing_field(stringify!($field)))
    };
    ($field:expr,$field_name:expr) => {
        $field.ok_or_else(|| bendy::decoding::Error::missing_field($field_name))
    };
}
