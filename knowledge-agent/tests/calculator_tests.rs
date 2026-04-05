use knowledge_agent::calculate;

// ── Basic arithmetic ───────────────────────────────────────────────────

#[test]
fn add() {
    assert_eq!(calculate("2 + 3").unwrap(), 5.0);
}

#[test]
fn subtract() {
    assert_eq!(calculate("10 - 4").unwrap(), 6.0);
}

#[test]
fn multiply() {
    assert_eq!(calculate("6 * 7").unwrap(), 42.0);
}

#[test]
fn divide() {
    assert_eq!(calculate("15 / 4").unwrap(), 3.75);
}

#[test]
fn modulo() {
    assert_eq!(calculate("17 % 5").unwrap(), 2.0);
}

#[test]
fn power() {
    assert_eq!(calculate("2 ^ 10").unwrap(), 1024.0);
}

// ── Precedence ─────────────────────────────────────────────────────────

#[test]
fn precedence_mul_add() {
    assert_eq!(calculate("2 + 3 * 4").unwrap(), 14.0);
}

#[test]
fn precedence_parentheses() {
    assert_eq!(calculate("(2 + 3) * 4").unwrap(), 20.0);
}

#[test]
fn nested_parentheses() {
    assert_eq!(calculate("((2 + 3) * (4 - 1))").unwrap(), 15.0);
}

#[test]
fn power_right_associative() {
    // 2^3^2 = 2^(3^2) = 2^9 = 512
    assert_eq!(calculate("2 ^ 3 ^ 2").unwrap(), 512.0);
}

// ── Decimals ───────────────────────────────────────────────────────────

#[test]
fn decimal_multiply() {
    let result = calculate("15 * 1.08").unwrap();
    assert!((result - 16.2).abs() < 1e-10);
}

#[test]
fn decimal_divide() {
    let result = calculate("1 / 3").unwrap();
    assert!((result - 0.333333333333).abs() < 1e-6);
}

// ── Unary minus ────────────────────────────────────────────────────────

#[test]
fn unary_minus() {
    assert_eq!(calculate("-5 + 3").unwrap(), -2.0);
}

#[test]
fn unary_minus_in_parens() {
    assert_eq!(calculate("(-5) * 2").unwrap(), -10.0);
}

// ── Functions ──────────────────────────────────────────────────────────

#[test]
fn sqrt() {
    assert_eq!(calculate("sqrt(144)").unwrap(), 12.0);
}

#[test]
fn abs() {
    assert_eq!(calculate("abs(-42)").unwrap(), 42.0);
}

#[test]
fn sin_zero() {
    assert!((calculate("sin(0)").unwrap()).abs() < 1e-10);
}

#[test]
fn cos_zero() {
    assert!((calculate("cos(0)").unwrap() - 1.0).abs() < 1e-10);
}

#[test]
fn log_functions() {
    // log() and ln() are both natural log; log10() is base-10; log2() is base-2
    assert!((calculate("log(1)").unwrap() - 0.0).abs() < 1e-10);
    assert!((calculate("ln(1)").unwrap() - 0.0).abs() < 1e-10);
    assert!((calculate("log10(100)").unwrap() - 2.0).abs() < 1e-10);
    assert!((calculate("log2(8)").unwrap() - 3.0).abs() < 1e-10);
}

#[test]
fn ln() {
    assert!((calculate("ln(e)").unwrap() - 1.0).abs() < 1e-10);
}

#[test]
fn exp() {
    assert!((calculate("exp(1)").unwrap() - std::f64::consts::E).abs() < 1e-10);
}

#[test]
fn ceil() {
    assert_eq!(calculate("ceil(4.2)").unwrap(), 5.0);
}

#[test]
fn floor() {
    assert_eq!(calculate("floor(4.8)").unwrap(), 4.0);
}

#[test]
fn round() {
    assert_eq!(calculate("round(4.5)").unwrap(), 5.0);
}

#[test]
fn min_max() {
    assert_eq!(calculate("min(3, 7)").unwrap(), 3.0);
    assert_eq!(calculate("max(3, 7)").unwrap(), 7.0);
}

#[test]
fn pow_func() {
    assert_eq!(calculate("pow(2, 10)").unwrap(), 1024.0);
}

// ── Constants ──────────────────────────────────────────────────────────

#[test]
fn pi() {
    assert!((calculate("pi").unwrap() - std::f64::consts::PI).abs() < 1e-10);
}

#[test]
fn euler() {
    assert!((calculate("e").unwrap() - std::f64::consts::E).abs() < 1e-10);
}

#[test]
fn pi_in_expression() {
    let result = calculate("2 * pi * 5").unwrap();
    assert!((result - 2.0 * std::f64::consts::PI * 5.0).abs() < 1e-10);
}

// ── Error handling ─────────────────────────────────────────────────────

#[test]
fn division_by_zero() {
    assert!(calculate("1 / 0").is_err());
}

#[test]
fn modulo_by_zero() {
    assert!(calculate("5 % 0").is_err());
}

#[test]
fn unknown_function() {
    assert!(calculate("foo(1)").is_err());
}

#[test]
fn wrong_arg_count() {
    assert!(calculate("sqrt(1, 2)").is_err());
}

#[test]
fn missing_paren() {
    assert!(calculate("(2 + 3").is_err());
}

#[test]
fn empty_expression() {
    assert!(calculate("").is_err());
}

#[test]
fn sqrt_negative() {
    assert!(calculate("sqrt(-1)").is_err());
}

#[test]
fn log_negative() {
    assert!(calculate("log(-1)").is_err());
}

#[test]
fn ln_zero() {
    assert!(calculate("ln(0)").is_err());
}

// ── New functions (asin, acos, atan, factorial) ────────────────────────

#[test]
fn asin_acos_atan() {
    assert!((calculate("asin(0)").unwrap() - 0.0).abs() < 1e-10);
    assert!((calculate("acos(1)").unwrap() - 0.0).abs() < 1e-10);
    assert!((calculate("atan(0)").unwrap() - 0.0).abs() < 1e-10);
    assert!((calculate("atan2(1, 1)").unwrap() - std::f64::consts::PI / 4.0).abs() < 1e-10);
}

#[test]
fn asin_domain_error() {
    assert!(calculate("asin(2)").is_err());
    assert!(calculate("acos(-2)").is_err());
}

#[test]
fn factorial_basic() {
    assert!((calculate("factorial(0)").unwrap() - 1.0).abs() < 1e-10);
    assert!((calculate("factorial(5)").unwrap() - 120.0).abs() < 1e-10);
    assert!((calculate("factorial(10)").unwrap() - 3628800.0).abs() < 1e-6);
}

#[test]
fn factorial_error_cases() {
    assert!(calculate("factorial(-1)").is_err());
    assert!(calculate("factorial(1.5)").is_err());
    assert!(calculate("factorial(171)").is_err());
}

// ── New utility functions ──────────────────────────────────────────────

#[test]
fn degrees_radians() {
    assert!((calculate("degrees(pi)").unwrap() - 180.0).abs() < 1e-10);
    assert!((calculate("radians(180)").unwrap() - std::f64::consts::PI).abs() < 1e-10);
    assert!((calculate("sin(radians(90))").unwrap() - 1.0).abs() < 1e-10);
}

#[test]
fn hypot_fn() {
    assert!((calculate("hypot(3, 4)").unwrap() - 5.0).abs() < 1e-10);
}

#[test]
fn sign_fn() {
    assert_eq!(calculate("sign(5)").unwrap(), 1.0);
    assert_eq!(calculate("sign(-3)").unwrap(), -1.0);
    assert_eq!(calculate("sign(0)").unwrap(), 0.0);
}

#[test]
fn trunc_fn() {
    assert_eq!(calculate("trunc(4.9)").unwrap(), 4.0);
    assert_eq!(calculate("trunc(-4.9)").unwrap(), -4.0);
}

#[test]
fn gcd_lcm() {
    assert_eq!(calculate("gcd(12, 8)").unwrap(), 4.0);
    assert_eq!(calculate("lcm(4, 6)").unwrap(), 12.0);
    assert!(calculate("gcd(1.5, 3)").is_err());
}

#[test]
fn log_two_arg() {
    // log(x, base) = log_base(x)
    assert!((calculate("log(8, 2)").unwrap() - 3.0).abs() < 1e-10);
    assert!((calculate("log(100, 10)").unwrap() - 2.0).abs() < 1e-10);
    assert!(calculate("log(8, 1)").is_err());   // base = 1 invalid
    assert!(calculate("log(8, -2)").is_err());  // negative base
}

// ── Complex expressions ────────────────────────────────────────────────

#[test]
fn complex_nested() {
    // sqrt(pow(3,2) + pow(4,2)) = sqrt(9+16) = sqrt(25) = 5
    assert_eq!(calculate("sqrt(pow(3,2) + pow(4,2))").unwrap(), 5.0);
}

#[test]
fn scientific_notation() {
    assert_eq!(calculate("1.5e3 + 500").unwrap(), 2000.0);
}
