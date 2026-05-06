use ailoy::{
    datatype::Value,
    message::ToolDescBuilder,
    to_value,
    tool::{ToolFactory, ToolFunc},
};

// ── Public entry point ─────────────────────────────────────────────────

pub fn build_calculate_tool() -> ToolFactory {
    let desc = ToolDescBuilder::new("calculate")
        .description(
            "Evaluate a mathematical expression and return the numeric result. \
             Supports: +, -, *, /, %, ^ (power), parentheses, \
             functions: sqrt, abs, sin, cos, tan, asin, acos, atan, atan2, \
             log/ln (natural log), log(x, base), log2, log10, exp, \
             ceil, floor, round, trunc, sign, degrees, radians, \
             hypot, gcd, lcm, min, max, pow, factorial. \
             Constants: pi, e. \
             Use this for any arithmetic that appears in your reasoning.",
        )
        .parameters(to_value!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Mathematical expression (e.g. \"15 * 1.08\", \"sqrt(144)\", \"2^10\")"
                }
            },
            "required": ["expression"]
        }))
        .build();

    let func = ToolFunc::new(move |args: Value| async move {
        let expression = match args.pointer("/expression").and_then(|v: &Value| v.as_str()) {
            Some(e) => e.to_string(),
            None => return to_value!({"error": "missing required parameter: expression"}),
        };
        match calculate(&expression) {
            Ok(result) => to_value!({"result": result, "expression": expression}),
            Err(e) => to_value!({"error": e, "expression": expression}),
        }
    });

    ToolFactory::simple(desc, func)
}

// ── Core evaluator ─────────────────────────────────────────────────────

fn calculate(expr: &str) -> Result<f64, String> {
    let tokens = tokenize(expr)?;
    let mut pos = 0;
    let result = parse_expr(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(format!("unexpected token: {:?}", tokens[pos]));
    }
    Ok(result)
}

// ── Tokenizer ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Token {
    Num(f64),
    Op(char),
    LParen,
    RParen,
    Comma,
    Func(String),
}

fn tokenize(expr: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = expr.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' => i += 1,
            '+' | '-' => {
                let is_unary = tokens.is_empty()
                    || matches!(
                        tokens.last(),
                        Some(Token::Op(_)) | Some(Token::LParen) | Some(Token::Comma)
                    );
                if is_unary && chars[i] == '-' {
                    i += 1;
                    if i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                        let start = i;
                        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                            i += 1;
                        }
                        let num_str: String = chars[start..i].iter().collect();
                        let num: f64 = num_str
                            .parse()
                            .map_err(|_| format!("invalid number: -{}", num_str))?;
                        tokens.push(Token::Num(-num));
                    } else if i < chars.len() && (chars[i] == '(' || chars[i].is_alphabetic()) {
                        tokens.push(Token::Num(-1.0));
                        tokens.push(Token::Op('*'));
                    } else {
                        return Err("unexpected '-'".to_string());
                    }
                } else if is_unary && chars[i] == '+' {
                    i += 1;
                } else {
                    tokens.push(Token::Op(chars[i]));
                    i += 1;
                }
            }
            '*' | '/' | '%' | '^' => {
                tokens.push(Token::Op(chars[i]));
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // scientific notation (e.g. 1.5e10)
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    i += 1;
                    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                        i += 1;
                    }
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let num_str: String = chars[start..i].iter().collect();
                let num: f64 = num_str
                    .parse()
                    .map_err(|_| format!("invalid number: {}", num_str))?;
                tokens.push(Token::Num(num));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let name: String = chars[start..i].iter().collect();
                match name.as_str() {
                    "pi" => tokens.push(Token::Num(std::f64::consts::PI)),
                    "e" => tokens.push(Token::Num(std::f64::consts::E)),
                    _ => tokens.push(Token::Func(name)),
                }
            }
            other => return Err(format!("unexpected character: '{}'", other)),
        }
    }

    Ok(tokens)
}

// ── Recursive descent parser ───────────────────────────────────────────

fn parse_expr(tokens: &[Token], pos: &mut usize) -> Result<f64, String> {
    let mut left = parse_term(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Op('+') => {
                *pos += 1;
                left += parse_term(tokens, pos)?;
            }
            Token::Op('-') => {
                *pos += 1;
                left -= parse_term(tokens, pos)?;
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_term(tokens: &[Token], pos: &mut usize) -> Result<f64, String> {
    let mut left = parse_power(tokens, pos)?;
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::Op('*') => {
                *pos += 1;
                left *= parse_power(tokens, pos)?;
            }
            Token::Op('/') => {
                *pos += 1;
                let right = parse_power(tokens, pos)?;
                if right == 0.0 {
                    return Err("division by zero".to_string());
                }
                left /= right;
            }
            Token::Op('%') => {
                *pos += 1;
                let right = parse_power(tokens, pos)?;
                if right == 0.0 {
                    return Err("modulo by zero".to_string());
                }
                left %= right;
            }
            _ => break,
        }
    }
    Ok(left)
}

// right-associative
fn parse_power(tokens: &[Token], pos: &mut usize) -> Result<f64, String> {
    let base = parse_atom(tokens, pos)?;
    if *pos < tokens.len() {
        if let Token::Op('^') = &tokens[*pos] {
            *pos += 1;
            let exp = parse_power(tokens, pos)?;
            return Ok(base.powf(exp));
        }
    }
    Ok(base)
}

fn parse_atom(tokens: &[Token], pos: &mut usize) -> Result<f64, String> {
    if *pos >= tokens.len() {
        return Err("unexpected end of expression".to_string());
    }

    match &tokens[*pos] {
        Token::Num(n) => {
            let val = *n;
            *pos += 1;
            Ok(val)
        }
        Token::LParen => {
            *pos += 1;
            let val = parse_expr(tokens, pos)?;
            if *pos >= tokens.len() {
                return Err("missing closing parenthesis".to_string());
            }
            match &tokens[*pos] {
                Token::RParen => {
                    *pos += 1;
                    Ok(val)
                }
                _ => Err("expected closing parenthesis".to_string()),
            }
        }
        Token::Func(name) => {
            let name = name.clone();
            *pos += 1;
            if *pos >= tokens.len() {
                return Err(format!("expected '(' after function '{}'", name));
            }
            match &tokens[*pos] {
                Token::LParen => *pos += 1,
                _ => return Err(format!("expected '(' after function '{}'", name)),
            }
            let mut args = vec![parse_expr(tokens, pos)?];
            while *pos < tokens.len() {
                if let Token::Comma = &tokens[*pos] {
                    *pos += 1;
                    args.push(parse_expr(tokens, pos)?);
                } else {
                    break;
                }
            }
            if *pos >= tokens.len() {
                return Err(format!("missing ')' for function '{}'", name));
            }
            match &tokens[*pos] {
                Token::RParen => *pos += 1,
                _ => return Err(format!("expected ')' for function '{}'", name)),
            }
            eval_func(&name, &args)
        }
        other => Err(format!("unexpected token: {:?}", other)),
    }
}

// ── Function evaluation ────────────────────────────────────────────────

fn eval_func(name: &str, args: &[f64]) -> Result<f64, String> {
    match name {
        "sqrt" => {
            ensure_args(name, args, 1)?;
            if args[0] < 0.0 {
                return Err("sqrt of negative number".to_string());
            }
            Ok(args[0].sqrt())
        }
        "abs" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].abs())
        }
        "sin" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].sin())
        }
        "cos" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].cos())
        }
        "tan" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].tan())
        }
        "asin" => {
            ensure_args(name, args, 1)?;
            if args[0] < -1.0 || args[0] > 1.0 {
                return Err("asin domain error: argument must be in [-1, 1]".to_string());
            }
            Ok(args[0].asin())
        }
        "acos" => {
            ensure_args(name, args, 1)?;
            if args[0] < -1.0 || args[0] > 1.0 {
                return Err("acos domain error: argument must be in [-1, 1]".to_string());
            }
            Ok(args[0].acos())
        }
        "atan" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].atan())
        }
        "atan2" => {
            ensure_args(name, args, 2)?;
            Ok(args[0].atan2(args[1]))
        }
        "ln" => {
            ensure_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("ln of non-positive number".to_string());
            }
            Ok(args[0].ln())
        }
        "log" => match args.len() {
            1 => {
                if args[0] <= 0.0 {
                    return Err("log of non-positive number".to_string());
                }
                Ok(args[0].ln())
            }
            2 => {
                if args[0] <= 0.0 {
                    return Err("log of non-positive number".to_string());
                }
                if args[1] <= 0.0 || args[1] == 1.0 {
                    return Err("log base must be positive and not 1".to_string());
                }
                Ok(args[0].ln() / args[1].ln())
            }
            _ => Err(format!(
                "log() expects 1 or 2 arguments, got {}",
                args.len()
            )),
        },
        "log10" => {
            ensure_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("log10 of non-positive number".to_string());
            }
            Ok(args[0].log10())
        }
        "log2" => {
            ensure_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("log2 of non-positive number".to_string());
            }
            Ok(args[0].log2())
        }
        "exp" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].exp())
        }
        "ceil" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].ceil())
        }
        "floor" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].floor())
        }
        "round" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].round())
        }
        "trunc" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].trunc())
        }
        "sign" => {
            ensure_args(name, args, 1)?;
            Ok(if args[0] > 0.0 {
                1.0
            } else if args[0] < 0.0 {
                -1.0
            } else {
                0.0
            })
        }
        "degrees" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].to_degrees())
        }
        "radians" => {
            ensure_args(name, args, 1)?;
            Ok(args[0].to_radians())
        }
        "hypot" => {
            ensure_args(name, args, 2)?;
            Ok(args[0].hypot(args[1]))
        }
        "pow" => {
            ensure_args(name, args, 2)?;
            Ok(args[0].powf(args[1]))
        }
        "min" => {
            if args.len() < 2 {
                return Err(format!(
                    "min() requires at least 2 arguments, got {}",
                    args.len()
                ));
            }
            Ok(args.iter().cloned().fold(f64::INFINITY, f64::min))
        }
        "max" => {
            if args.len() < 2 {
                return Err(format!(
                    "max() requires at least 2 arguments, got {}",
                    args.len()
                ));
            }
            Ok(args.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
        }
        "gcd" => {
            ensure_args(name, args, 2)?;
            if args[0].fract() != 0.0 || args[1].fract() != 0.0 {
                return Err("gcd requires integer arguments".to_string());
            }
            let mut x = (args[0].abs() as u64, args[1].abs() as u64);
            while x.1 != 0 {
                x = (x.1, x.0 % x.1);
            }
            Ok(x.0 as f64)
        }
        "lcm" => {
            ensure_args(name, args, 2)?;
            if args[0].fract() != 0.0 || args[1].fract() != 0.0 {
                return Err("lcm requires integer arguments".to_string());
            }
            if args[0] == 0.0 || args[1] == 0.0 {
                return Ok(0.0);
            }
            let (a, b) = (args[0].abs() as u64, args[1].abs() as u64);
            let mut gcd = (a, b);
            while gcd.1 != 0 {
                gcd = (gcd.1, gcd.0 % gcd.1);
            }
            Ok((a / gcd.0 * b) as f64)
        }
        "factorial" => {
            ensure_args(name, args, 1)?;
            let n = args[0];
            if n < 0.0 || n.fract() != 0.0 {
                return Err("factorial requires a non-negative integer".to_string());
            }
            if n > 170.0 {
                return Err("factorial argument too large (max 170)".to_string());
            }
            let mut result = 1.0f64;
            for i in 2..=(n as u64) {
                result *= i as f64;
            }
            Ok(result)
        }
        _ => Err(format!("unknown function: {}", name)),
    }
}

fn ensure_args(name: &str, args: &[f64], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        Err(format!(
            "{}() expects {} argument(s), got {}",
            name,
            expected,
            args.len()
        ))
    } else {
        Ok(())
    }
}
