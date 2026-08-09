//! Number conversion: JSON5 semantics — hex integers, `Infinity`/`NaN`, signs, and integer
//! preservation.

use strata_json5::{Json5Error, Json5Number, Json5Resolver, NumberFormat};

fn number(input: &str) -> Result<Json5Number, Json5Error> {
    let document = Json5Resolver::new().resolve_str(input)?;
    let Some(number) = document.root().as_number() else {
        panic!("expected {input:?} to resolve to a number");
    };

    Ok(number)
}

#[test]
fn decimal_integers_preserve_identity() -> Result<(), Json5Error> {
    assert_eq!(number("0")?, Json5Number::U64(0));
    assert_eq!(number("18446744073709551615")?, Json5Number::U64(u64::MAX));
    assert_eq!(number("-9223372036854775808")?, Json5Number::I64(i64::MIN));

    Ok(())
}

#[test]
fn explicit_plus_signs_preserve_integer_identity() -> Result<(), Json5Error> {
    assert_eq!(number("+42")?, Json5Number::U64(42));
    assert_eq!(number("+0")?, Json5Number::U64(0));
    assert_eq!(number("+0x10")?, Json5Number::U64(16));

    Ok(())
}

#[test]
fn leading_and_trailing_decimal_points_convert() -> Result<(), Json5Error> {
    assert_eq!(number(".5")?, Json5Number::F64(0.5));
    assert_eq!(number("-.5")?, Json5Number::F64(-0.5));
    assert_eq!(number("5.")?, Json5Number::F64(5.0));
    assert_eq!(number("5.e2")?, Json5Number::F64(500.0));

    Ok(())
}

#[test]
fn fractions_exponents_and_overflow_convert_to_f64() -> Result<(), Json5Error> {
    assert_eq!(number("2.5e-3")?, Json5Number::F64(2.5e-3));
    assert_eq!(number("-0.5")?, Json5Number::F64(-0.5));
    assert_eq!(
        number("18446744073709551616")?,
        Json5Number::F64(18_446_744_073_709_551_616.0)
    );

    Ok(())
}

#[test]
fn hex_integers_preserve_integer_identity() -> Result<(), Json5Error> {
    assert_eq!(number("0x0")?, Json5Number::U64(0));
    assert_eq!(number("0xdecaf")?, Json5Number::U64(0x000D_ECAF));
    assert_eq!(number("0XABCDEF")?, Json5Number::U64(0x00AB_CDEF));
    assert_eq!(number("0xFFFFFFFFFFFFFFFF")?, Json5Number::U64(u64::MAX));
    assert_eq!(number("-0x10")?, Json5Number::I64(-16));
    assert_eq!(number("-0x8000000000000000")?, Json5Number::I64(i64::MIN));

    Ok(())
}

#[test]
fn oversized_hex_magnitudes_fall_back_to_f64() -> Result<(), Json5Error> {
    // Seventeen F digits: 2^68 - 1, far past u64 — approximates as a float.
    assert_eq!(
        number("0xFFFFFFFFFFFFFFFFF")?,
        Json5Number::F64(295_147_905_179_352_825_856.0)
    );
    // One past i64::MIN's magnitude.
    assert_eq!(
        number("-0x8000000000000001")?,
        Json5Number::F64(-9_223_372_036_854_775_809.0)
    );

    Ok(())
}

#[test]
fn infinity_converts_with_its_sign() -> Result<(), Json5Error> {
    assert_eq!(number("Infinity")?, Json5Number::F64(f64::INFINITY));
    assert_eq!(number("+Infinity")?, Json5Number::F64(f64::INFINITY));
    assert_eq!(number("-Infinity")?, Json5Number::F64(f64::NEG_INFINITY));

    Ok(())
}

#[test]
fn nan_converts_with_its_sign() -> Result<(), Json5Error> {
    for (input, negative) in [("NaN", false), ("+NaN", false), ("-NaN", true)] {
        let Json5Number::F64(value) = number(input)? else {
            panic!("expected an f64 NaN for {input:?}");
        };
        assert!(value.is_nan(), "input {input:?}");
        assert_eq!(value.is_sign_negative(), negative, "input {input:?}");
    }

    Ok(())
}

#[test]
fn negative_zeros_preserve_their_sign() -> Result<(), Json5Error> {
    for input in ["-0", "-0x0", "-0X00"] {
        let Json5Number::F64(value) = number(input)? else {
            panic!("expected -0 to resolve as a float for {input:?}");
        };
        assert_eq!(value.to_bits(), (-0.0f64).to_bits(), "input {input:?}");
    }

    Ok(())
}

fn format(input: &str) -> Result<Option<NumberFormat>, Json5Error> {
    Ok(Json5Resolver::new()
        .resolve_str(input)?
        .root()
        .number_format())
}

#[test]
fn number_formats_surface_the_written_form() -> Result<(), Json5Error> {
    assert_eq!(format("42")?, Some(NumberFormat::Integer));
    assert_eq!(format(".5")?, Some(NumberFormat::Decimal));
    assert_eq!(format("1.23e+5")?, Some(NumberFormat::Exponential));
    assert_eq!(format("-0x10")?, Some(NumberFormat::Hex));
    assert_eq!(format("+Infinity")?, Some(NumberFormat::Infinity));
    assert_eq!(format("NaN")?, Some(NumberFormat::NaN));
    assert_eq!(format("true")?, None);

    Ok(())
}
