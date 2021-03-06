use itertools::Itertools;
use pest::iterators::Pair;
use pest::Parser;
use std::borrow::Cow;
use std::rc::Rc;

use dhall_generated_parser::{DhallParser, Rule};

use crate::map::DupTreeMap;
use crate::ExprF::*;
use crate::*;

// This file consumes the parse tree generated by pest and turns it into
// our own AST. All those custom macros should eventually moved into
// their own crate because they are quite general and useful. For now they
// are here and hopefully you can figure out how they work.

type ParsedExpr = Expr<Span, Import>;
type ParsedSubExpr = SubExpr<Span, Import>;
type ParsedText = InterpolatedText<SubExpr<Span, Import>>;
type ParsedTextContents = InterpolatedTextContents<SubExpr<Span, Import>>;

pub type ParseError = pest::error::Error<Rule>;

pub type ParseResult<T> = Result<T, ParseError>;

fn unspanned(x: ParsedExpr) -> ParsedSubExpr {
    SubExpr::from_expr_no_note(x)
}

#[derive(Debug, Clone)]
pub struct Span {
    input: Rc<str>,
    /// # Safety
    ///
    /// Must be a valid character boundary index into `input`.
    start: usize,
    /// # Safety
    ///
    /// Must be a valid character boundary index into `input`.
    end: usize,
}

impl Span {
    fn make(input: Rc<str>, sp: pest::Span) -> Self {
        Span {
            input,
            start: sp.start(),
            end: sp.end(),
        }
    }
}

fn spanned(span: Span, x: ParsedExpr) -> ParsedSubExpr {
    SubExpr::new(x, span)
}

#[derive(Debug)]
enum Either<A, B> {
    Left(A),
    Right(B),
}

impl crate::Builtin {
    pub fn parse(s: &str) -> Option<Self> {
        use crate::Builtin::*;
        match s {
            "Bool" => Some(Bool),
            "Natural" => Some(Natural),
            "Integer" => Some(Integer),
            "Double" => Some(Double),
            "Text" => Some(Text),
            "List" => Some(List),
            "Optional" => Some(Optional),
            "None" => Some(OptionalNone),
            "Natural/build" => Some(NaturalBuild),
            "Natural/fold" => Some(NaturalFold),
            "Natural/isZero" => Some(NaturalIsZero),
            "Natural/even" => Some(NaturalEven),
            "Natural/odd" => Some(NaturalOdd),
            "Natural/toInteger" => Some(NaturalToInteger),
            "Natural/show" => Some(NaturalShow),
            "Integer/toDouble" => Some(IntegerToDouble),
            "Integer/show" => Some(IntegerShow),
            "Double/show" => Some(DoubleShow),
            "List/build" => Some(ListBuild),
            "List/fold" => Some(ListFold),
            "List/length" => Some(ListLength),
            "List/head" => Some(ListHead),
            "List/last" => Some(ListLast),
            "List/indexed" => Some(ListIndexed),
            "List/reverse" => Some(ListReverse),
            "Optional/fold" => Some(OptionalFold),
            "Optional/build" => Some(OptionalBuild),
            "Text/show" => Some(TextShow),
            _ => None,
        }
    }
}

pub fn custom_parse_error(pair: &Pair<Rule>, msg: String) -> ParseError {
    let msg =
        format!("{} while matching on:\n{}", msg, debug_pair(pair.clone()));
    let e = pest::error::ErrorVariant::CustomError { message: msg };
    pest::error::Error::new_from_span(e, pair.as_span())
}

fn debug_pair(pair: Pair<Rule>) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    fn aux(s: &mut String, indent: usize, prefix: String, pair: Pair<Rule>) {
        let indent_str = "| ".repeat(indent);
        let rule = pair.as_rule();
        let contents = pair.as_str();
        let mut inner = pair.into_inner();
        let mut first = true;
        while let Some(p) = inner.next() {
            if first {
                first = false;
                let last = inner.peek().is_none();
                if last && p.as_str() == contents {
                    let prefix = format!("{}{:?} > ", prefix, rule);
                    aux(s, indent, prefix, p);
                    continue;
                } else {
                    writeln!(
                        s,
                        r#"{}{}{:?}: "{}""#,
                        indent_str, prefix, rule, contents
                    )
                    .unwrap();
                }
            }
            aux(s, indent + 1, "".into(), p);
        }
        if first {
            writeln!(
                s,
                r#"{}{}{:?}: "{}""#,
                indent_str, prefix, rule, contents
            )
            .unwrap();
        }
    }
    aux(&mut s, 0, "".into(), pair);
    s
}

macro_rules! make_parser {
    (@pattern, rule, $name:ident) => (Rule::$name);
    (@pattern, token_rule, $name:ident) => (Rule::$name);
    (@pattern, rule_group, $name:ident) => (_);
    (@filter, rule) => (true);
    (@filter, token_rule) => (true);
    (@filter, rule_group) => (false);

    (@body,
        ($($things:tt)*),
        rule!( $name:ident<$o:ty>; $($args:tt)* )
    ) => (
        make_parser!(@body,
            ($($things)*),
            rule!( $name<$o> as $name; $($args)* )
        )
    );
    (@body,
        ($_input:expr, $pair:expr, $_children:expr),
        rule!(
            $name:ident<$o:ty>
            as $group:ident;
            captured_str!($x:pat) => $body:expr
        )
    ) => ({
        let $x = $pair.as_str();
        let res: $o = $body;
        Ok(ParsedValue::$group(res))
    });
    (@body,
        ($_input:expr, $_pair:expr, $children:expr),
        rule!(
            $name:ident<$o:ty>
            as $group:ident;
            children!( $( [$($args:tt)*] => $body:expr ),* $(,)* )
        )
    ) => ({
        #[allow(unused_imports)]
        use ParsedValue::*;
        #[allow(unreachable_code)]
        let res: $o = improved_slice_patterns::match_vec!($children;
            $( [$($args)*] => $body, )*
            [x..] => Err(
                format!("Unexpected children: {:?}", x.collect::<Vec<_>>())
            )?,
        ).map_err(|_| -> String { unreachable!() })?;
        Ok(ParsedValue::$group(res))
    });
    (@body,
        ($input:expr, $pair:expr, $children:expr),
        rule!(
            $name:ident<$o:ty>
            as $group:ident;
            $span:ident;
            $($args:tt)*
        )
    ) => ({
        let $span = Span::make($input, $pair.as_span());
        make_parser!(@body,
            ($input, $pair, $children),
            rule!(
                $name<$o>
                as $group;
                $($args)*
            )
        )
    });
    (@body,
        ($($things:tt)*),
        token_rule!($name:ident<$o:ty>)
    ) => ({
        Ok(ParsedValue::$name(()))
    });
    (@body, ($($things:tt)*), rule_group!( $name:ident<$o:ty> )) => (
        unreachable!()
    );

    ($( $submac:ident!( $name:ident<$o:ty> $($args:tt)* ); )*) => (
        #[allow(non_camel_case_types, dead_code, clippy::large_enum_variant)]
        #[derive(Debug)]
        enum ParsedValue<'a> {
            $( $name($o), )*
        }

        fn parse_any<'a>(
            input: Rc<str>,
            pair: Pair<'a, Rule>,
            children: Vec<ParsedValue<'a>>,
        ) -> Result<ParsedValue<'a>, String> {
            match pair.as_rule() {
                $(
                    make_parser!(@pattern, $submac, $name)
                    if make_parser!(@filter, $submac)
                    => make_parser!(@body, (input, pair, children),
                                           $submac!( $name<$o> $($args)* ))
                    ,
                )*
                r => Err(format!("Unexpected {:?}", r)),
            }
        }
    );
}

// Non-recursive implementation to avoid stack overflows
fn do_parse<'a>(
    input: Rc<str>,
    initial_pair: Pair<'a, Rule>,
) -> ParseResult<ParsedValue<'a>> {
    enum StackFrame<'a> {
        Unprocessed(Pair<'a, Rule>),
        Processed(Pair<'a, Rule>, usize),
    }
    use StackFrame::*;
    let mut pairs_stack: Vec<StackFrame> =
        vec![Unprocessed(initial_pair.clone())];
    let mut values_stack: Vec<ParsedValue> = vec![];
    while let Some(p) = pairs_stack.pop() {
        match p {
            Unprocessed(mut pair) => loop {
                let mut pairs: Vec<_> = pair.clone().into_inner().collect();
                let n_children = pairs.len();
                if n_children == 1 && can_be_shortcutted(pair.as_rule()) {
                    pair = pairs.pop().unwrap();
                    continue;
                } else {
                    pairs_stack.push(Processed(pair, n_children));
                    pairs_stack
                        .extend(pairs.into_iter().map(StackFrame::Unprocessed));
                    break;
                }
            },
            Processed(pair, n) => {
                let mut children: Vec<_> =
                    values_stack.split_off(values_stack.len() - n);
                children.reverse();
                let val = match parse_any(input.clone(), pair.clone(), children)
                {
                    Ok(v) => v,
                    Err(msg) => Err(custom_parse_error(&pair, msg))?,
                };
                values_stack.push(val);
            }
        }
    }
    Ok(values_stack.pop().unwrap())
}

// List of rules that can be shortcutted if they have a single child
fn can_be_shortcutted(rule: Rule) -> bool {
    use Rule::*;
    match rule {
        expression
        | import_alt_expression
        | or_expression
        | plus_expression
        | text_append_expression
        | list_append_expression
        | and_expression
        | combine_expression
        | prefer_expression
        | combine_types_expression
        | times_expression
        | equal_expression
        | not_equal_expression
        | application_expression
        | first_application_expression
        | selector_expression
        | annotated_expression => true,
        _ => false,
    }
}

// Trim the shared indent off of a vec of lines, as defined by the Dhall semantics of multiline
// literals.
fn trim_indent(lines: &mut Vec<ParsedText>) {
    let is_indent = |c: char| c == ' ' || c == '\t';

    // There is at least one line so this is safe
    let last_line_head = lines.last().unwrap().head();
    let indent_chars = last_line_head
        .char_indices()
        .take_while(|(_, c)| is_indent(*c));
    let mut min_indent_idx = match indent_chars.last() {
        Some((i, _)) => i,
        // If there is no indent char, then no indent needs to be stripped
        None => return,
    };

    for line in lines.iter() {
        // Ignore empty lines
        if line.is_empty() {
            continue;
        }
        // Take chars from line while they match the current minimum indent.
        let indent_chars = last_line_head[0..=min_indent_idx]
            .char_indices()
            .zip(line.head().chars())
            .take_while(|((_, c1), c2)| c1 == c2);
        match indent_chars.last() {
            Some(((i, _), _)) => min_indent_idx = i,
            // If there is no indent char, then no indent needs to be stripped
            None => return,
        };
    }

    // Remove the shared indent from non-empty lines
    for line in lines.iter_mut() {
        if !line.is_empty() {
            line.head_mut().replace_range(0..=min_indent_idx, "");
        }
    }
}

make_parser! {
    token_rule!(EOI<()>);

    rule!(simple_label<Label>;
        captured_str!(s) => Label::from(s.trim().to_owned())
    );
    rule!(quoted_label<Label>;
        captured_str!(s) => Label::from(s.trim().to_owned())
    );
    rule!(label<Label>; children!(
        [simple_label(l)] => l,
        [quoted_label(l)] => l,
    ));

    rule!(double_quote_literal<ParsedText>; children!(
        [double_quote_chunk(chunks)..] => {
            chunks.collect()
        }
    ));

    rule!(double_quote_chunk<ParsedTextContents>; children!(
        [interpolation(e)] => {
            InterpolatedTextContents::Expr(e)
        },
        [double_quote_escaped(s)] => {
            InterpolatedTextContents::Text(s)
        },
        [double_quote_char(s)] => {
            InterpolatedTextContents::Text(s.to_owned())
        },
    ));
    rule!(double_quote_escaped<String>;
        captured_str!(s) => {
            match s {
                "\"" => "\"".to_owned(),
                "$" => "$".to_owned(),
                "\\" => "\\".to_owned(),
                "/" => "/".to_owned(),
                "b" => "\u{0008}".to_owned(),
                "f" => "\u{000C}".to_owned(),
                "n" => "\n".to_owned(),
                "r" => "\r".to_owned(),
                "t" => "\t".to_owned(),
                _ => {
                    // "uXXXX"
                    use std::convert::TryFrom;
                    let c = u16::from_str_radix(&s[1..5], 16).unwrap();
                    let c = char::try_from(u32::from(c)).unwrap();
                    std::iter::once(c).collect()
                }
            }
        }
    );
    rule!(double_quote_char<&'a str>;
        captured_str!(s) => s
    );

    rule!(single_quote_literal<ParsedText>; children!(
        [single_quote_continue(lines)] => {
            let newline: ParsedText = "\n".to_string().into();

            let mut lines: Vec<ParsedText> = lines
                .into_iter()
                .rev()
                .map(|l| l.into_iter().rev().collect::<ParsedText>())
                .collect();

            trim_indent(&mut lines);

            lines
                .into_iter()
                .intersperse(newline)
                .flat_map(InterpolatedText::into_iter)
                .collect::<ParsedText>()
        }
    ));
    rule!(single_quote_char<&'a str>;
        captured_str!(s) => s
    );
    rule!(escaped_quote_pair<&'a str> as single_quote_char;
        captured_str!(_) => "''"
    );
    rule!(escaped_interpolation<&'a str> as single_quote_char;
        captured_str!(_) => "${"
    );
    rule!(interpolation<ParsedSubExpr>; children!(
        [expression(e)] => e
    ));

    // Returns a vec of lines in reversed order, where each line is also in reversed order.
    rule!(single_quote_continue<Vec<Vec<ParsedTextContents>>>; children!(
        [interpolation(c), single_quote_continue(lines)] => {
            let c = InterpolatedTextContents::Expr(c);
            let mut lines = lines;
            lines.last_mut().unwrap().push(c);
            lines
        },
        [single_quote_char("\n"), single_quote_continue(lines)] => {
            let mut lines = lines;
            lines.push(vec![]);
            lines
        },
        [single_quote_char(c), single_quote_continue(lines)] => {
            // TODO: don't allocate for every char
            let c = InterpolatedTextContents::Text(c.to_owned());
            let mut lines = lines;
            lines.last_mut().unwrap().push(c);
            lines
        },
        [] => {
            vec![vec![]]
        },
    ));

    rule!(builtin<ParsedSubExpr>; span;
        captured_str!(s) => {
            spanned(span, match crate::Builtin::parse(s) {
                Some(b) => Builtin(b),
                None => match s {
                    "True" => BoolLit(true),
                    "False" => BoolLit(false),
                    "Type" => Const(crate::Const::Type),
                    "Kind" => Const(crate::Const::Kind),
                    "Sort" => Const(crate::Const::Sort),
                    _ => Err(
                        format!("Unrecognized builtin: '{}'", s)
                    )?,
                }
            })
        }
    );

    token_rule!(NaN<()>);
    token_rule!(minus_infinity_literal<()>);
    token_rule!(plus_infinity_literal<()>);

    rule!(numeric_double_literal<core::Double>;
        captured_str!(s) => {
            let s = s.trim();
            match s.parse::<f64>() {
                Ok(x) if x.is_infinite() =>
                    Err(format!("Overflow while parsing double literal '{}'", s))?,
                Ok(x) => NaiveDouble::from(x),
                Err(e) => Err(format!("{}", e))?,
            }
        }
    );

    rule!(double_literal<core::Double>; children!(
        [numeric_double_literal(n)] => n,
        [minus_infinity_literal(n)] => std::f64::NEG_INFINITY.into(),
        [plus_infinity_literal(n)] => std::f64::INFINITY.into(),
        [NaN(n)] => std::f64::NAN.into(),
    ));

    rule!(natural_literal<core::Natural>;
        captured_str!(s) => {
            s.trim()
                .parse()
                .map_err(|e| format!("{}", e))?
        }
    );

    rule!(integer_literal<core::Integer>;
        captured_str!(s) => {
            s.trim()
                .parse()
                .map_err(|e| format!("{}", e))?
        }
    );

    rule!(identifier<ParsedSubExpr> as expression; span; children!(
        [variable(v)] => {
            spanned(span, Var(v))
        },
        [builtin(e)] => e,
    ));

    rule!(variable<V<Label>>; children!(
        [label(l), natural_literal(idx)] => {
            V(l, idx)
        },
        [label(l)] => {
            V(l, 0)
        },
    ));

    rule!(unquoted_path_component<&'a str>; captured_str!(s) => s);
    rule!(quoted_path_component<&'a str>; captured_str!(s) => s);
    rule!(path_component<String>; children!(
        [unquoted_path_component(s)] => {
            percent_encoding::percent_decode(s.as_bytes())
                .decode_utf8_lossy()
                .into_owned()
        },
        [quoted_path_component(s)] => s.to_string(),
    ));
    rule!(path<Vec<String>>; children!(
        [path_component(components)..] => {
            components.collect()
        }
    ));

    rule_group!(local<(FilePrefix, Vec<String>)>);

    rule!(parent_path<(FilePrefix, Vec<String>)> as local; children!(
        [path(p)] => (FilePrefix::Parent, p)
    ));
    rule!(here_path<(FilePrefix, Vec<String>)> as local; children!(
        [path(p)] => (FilePrefix::Here, p)
    ));
    rule!(home_path<(FilePrefix, Vec<String>)> as local; children!(
        [path(p)] => (FilePrefix::Home, p)
    ));
    rule!(absolute_path<(FilePrefix, Vec<String>)> as local; children!(
        [path(p)] => (FilePrefix::Absolute, p)
    ));

    rule!(scheme<Scheme>; captured_str!(s) => match s {
        "http" => Scheme::HTTP,
        "https" => Scheme::HTTPS,
        _ => unreachable!(),
    });

    rule!(http_raw<URL>; children!(
        [scheme(sch), authority(auth), path(p)] => URL {
            scheme: sch,
            authority: auth,
            path: p,
            query: None,
            headers: None,
        },
        [scheme(sch), authority(auth), path(p), query(q)] => URL {
            scheme: sch,
            authority: auth,
            path: p,
            query: Some(q),
            headers: None,
        },
    ));

    rule!(authority<String>; captured_str!(s) => s.to_owned());

    rule!(query<String>; captured_str!(s) => s.to_owned());

    rule!(http<URL>; children!(
        [http_raw(url)] => url,
        [http_raw(url), import_hashed(ih)] =>
            URL { headers: Some(Box::new(ih)), ..url },
    ));

    rule!(env<String>; children!(
        [bash_environment_variable(s)] => s,
        [posix_environment_variable(s)] => s,
    ));
    rule!(bash_environment_variable<String>; captured_str!(s) => s.to_owned());
    rule!(posix_environment_variable<String>; children!(
        [posix_environment_variable_character(chars)..] => {
            chars.collect()
        },
    ));
    rule!(posix_environment_variable_character<Cow<'a, str>>;
        captured_str!(s) => {
            match s {
                "\\\"" => Cow::Owned("\"".to_owned()),
                "\\\\" => Cow::Owned("\\".to_owned()),
                "\\a" =>  Cow::Owned("\u{0007}".to_owned()),
                "\\b" =>  Cow::Owned("\u{0008}".to_owned()),
                "\\f" =>  Cow::Owned("\u{000C}".to_owned()),
                "\\n" =>  Cow::Owned("\n".to_owned()),
                "\\r" =>  Cow::Owned("\r".to_owned()),
                "\\t" =>  Cow::Owned("\t".to_owned()),
                "\\v" =>  Cow::Owned("\u{000B}".to_owned()),
                _ => Cow::Borrowed(s)
            }
        }
    );

    token_rule!(missing<()>);

    rule!(import_type<ImportLocation>; children!(
        [missing(_)] => {
            ImportLocation::Missing
        },
        [env(e)] => {
            ImportLocation::Env(e)
        },
        [http(url)] => {
            ImportLocation::Remote(url)
        },
        [local((prefix, p))] => {
            ImportLocation::Local(prefix, p)
        },
    ));

    rule!(hash<Hash>; captured_str!(s) =>
        Hash {
            protocol: s.trim()[..6].to_owned(),
            hash: s.trim()[7..].to_owned(),
        }
    );

    rule!(import_hashed<ImportHashed>; children!(
        [import_type(location)] =>
            ImportHashed { location, hash: None },
        [import_type(location), hash(h)] =>
            ImportHashed { location, hash: Some(h) },
    ));

    token_rule!(Text<()>);

    rule!(import<ParsedSubExpr> as expression; span; children!(
        [import_hashed(location_hashed)] => {
            spanned(span, Embed(Import {
                mode: ImportMode::Code,
                location_hashed
            }))
        },
        [import_hashed(location_hashed), Text(_)] => {
            spanned(span, Embed(Import {
                mode: ImportMode::RawText,
                location_hashed
            }))
        },
    ));

    token_rule!(lambda<()>);
    token_rule!(forall<()>);
    token_rule!(arrow<()>);
    token_rule!(merge<()>);
    token_rule!(if_<()>);
    token_rule!(in_<()>);

    rule!(expression<ParsedSubExpr> as expression; span; children!(
        [lambda(()), label(l), expression(typ),
                arrow(()), expression(body)] => {
            spanned(span, Lam(l, typ, body))
        },
        [if_(()), expression(cond), expression(left), expression(right)] => {
            spanned(span, BoolIf(cond, left, right))
        },
        [let_binding(bindings).., in_(()), expression(final_expr)] => {
            bindings.rev().fold(
                final_expr,
                |acc, x| unspanned(Let(x.0, x.1, x.2, acc))
            )
        },
        [forall(()), label(l), expression(typ),
                arrow(()), expression(body)] => {
            spanned(span, Pi(l, typ, body))
        },
        [expression(typ), arrow(()), expression(body)] => {
            spanned(span, Pi("_".into(), typ, body))
        },
        [merge(()), expression(x), expression(y), expression(z)] => {
            spanned(span, Merge(x, y, Some(z)))
        },
        [expression(e)] => e,
    ));

    rule!(let_binding<(Label, Option<ParsedSubExpr>, ParsedSubExpr)>;
            children!(
        [label(name), expression(annot), expression(expr)] =>
            (name, Some(annot), expr),
        [label(name), expression(expr)] =>
            (name, None, expr),
    ));

    token_rule!(List<()>);
    token_rule!(Optional<()>);

    rule!(empty_collection<ParsedSubExpr> as expression; span; children!(
        [List(_), expression(t)] => {
            spanned(span, EmptyListLit(t))
        },
        [Optional(_), expression(t)] => {
            spanned(span, OldOptionalLit(None, t))
        },
    ));

    rule!(non_empty_optional<ParsedSubExpr> as expression; span; children!(
        [expression(x), Optional(_), expression(t)] => {
            spanned(span, OldOptionalLit(Some(x), t))
        }
    ));

    rule!(import_alt_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::ImportAlt;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(or_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::BoolOr;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(plus_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::NaturalPlus;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(text_append_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::TextAppend;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(list_append_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::ListAppend;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(and_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::BoolAnd;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(combine_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::RecursiveRecordMerge;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(prefer_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::RightBiasedRecordMerge;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(combine_types_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::RecursiveRecordTypeMerge;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(times_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::NaturalTimes;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(equal_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::BoolEQ;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));
    rule!(not_equal_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            let o = crate::BinOp::BoolNE;
            rest.fold(first, |acc, e| unspanned(BinOp(o, acc, e)))
        },
    ));

    rule!(annotated_expression<ParsedSubExpr> as expression; span; children!(
        [expression(e)] => e,
        [expression(e), expression(annot)] => {
            spanned(span, Annot(e, annot))
        },
    ));

    token_rule!(Some_<()>);

    rule!(application_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), expression(rest)..] => {
            rest.fold(first, |acc, e| unspanned(App(acc, e)))
        },
    ));

    rule!(first_application_expression<ParsedSubExpr> as expression; span;
            children!(
        [expression(e)] => e,
        [Some_(()), expression(e)] => {
            spanned(span, SomeLit(e))
        },
        [merge(()), expression(x), expression(y)] => {
            spanned(span, Merge(x, y, None))
        },
    ));

    rule!(selector_expression<ParsedSubExpr> as expression; children!(
        [expression(e)] => e,
        [expression(first), selector(rest)..] => {
            rest.fold(first, |acc, e| unspanned(match e {
                Either::Left(l) => Field(acc, l),
                Either::Right(ls) => Projection(acc, ls),
            }))
        }
    ));

    rule!(selector<Either<Label, Vec<Label>>>; children!(
        [label(l)] => Either::Left(l),
        [labels(ls)] => Either::Right(ls),
    ));

    rule!(labels<Vec<Label>>; children!(
        [label(ls)..] => ls.collect(),
    ));

    rule!(primitive_expression<ParsedSubExpr> as expression; span; children!(
        [double_literal(n)] => spanned(span, DoubleLit(n)),
        [natural_literal(n)] => spanned(span, NaturalLit(n)),
        [integer_literal(n)] => spanned(span, IntegerLit(n)),
        [double_quote_literal(s)] => spanned(span, TextLit(s)),
        [single_quote_literal(s)] => spanned(span, TextLit(s)),
        [expression(e)] => e,
    ));

    rule!(empty_record_literal<ParsedSubExpr> as expression; span;
        captured_str!(_) => spanned(span, RecordLit(Default::default()))
    );

    rule!(empty_record_type<ParsedSubExpr> as expression; span;
        captured_str!(_) => spanned(span, RecordType(Default::default()))
    );

    rule!(non_empty_record_type_or_literal<ParsedSubExpr> as expression; span;
          children!(
        [label(first_label), non_empty_record_type(rest)] => {
            let (first_expr, mut map) = rest;
            map.insert(first_label, first_expr);
            spanned(span, RecordType(map))
        },
        [label(first_label), non_empty_record_literal(rest)] => {
            let (first_expr, mut map) = rest;
            map.insert(first_label, first_expr);
            spanned(span, RecordLit(map))
        },
    ));

    rule!(non_empty_record_type
          <(ParsedSubExpr, DupTreeMap<Label, ParsedSubExpr>)>; children!(
        [expression(expr), record_type_entry(entries)..] => {
            (expr, entries.collect())
        }
    ));

    rule!(record_type_entry<(Label, ParsedSubExpr)>; children!(
        [label(name), expression(expr)] => (name, expr)
    ));

    rule!(non_empty_record_literal
          <(ParsedSubExpr, DupTreeMap<Label, ParsedSubExpr>)>; children!(
        [expression(expr), record_literal_entry(entries)..] => {
            (expr, entries.collect())
        }
    ));

    rule!(record_literal_entry<(Label, ParsedSubExpr)>; children!(
        [label(name), expression(expr)] => (name, expr)
    ));

    rule!(union_type_or_literal<ParsedSubExpr> as expression; span; children!(
        [empty_union_type(_)] => {
            spanned(span, UnionType(Default::default()))
        },
        [non_empty_union_type_or_literal((Some((l, e)), entries))] => {
            spanned(span, UnionLit(l, e, entries))
        },
        [non_empty_union_type_or_literal((None, entries))] => {
            spanned(span, UnionType(entries))
        },
    ));

    token_rule!(empty_union_type<()>);

    rule!(non_empty_union_type_or_literal
          <(Option<(Label, ParsedSubExpr)>,
            DupTreeMap<Label, Option<ParsedSubExpr>>)>;
            children!(
        [label(l), union_literal_variant_value((e, entries))] => {
            (Some((l, e)), entries)
        },
        [label(l), union_type_or_literal_variant_type((e, rest))] => {
            let (x, mut entries) = rest;
            entries.insert(l, e);
            (x, entries)
        },
    ));

    rule!(union_literal_variant_value
          <(ParsedSubExpr, DupTreeMap<Label, Option<ParsedSubExpr>>)>;
            children!(
        [expression(e), union_type_entry(entries)..] => {
            (e, entries.collect())
        },
    ));

    rule!(union_type_entry<(Label, Option<ParsedSubExpr>)>; children!(
        [label(name), expression(expr)] => (name, Some(expr)),
        [label(name)] => (name, None),
    ));

    // TODO: unary union variants
    rule!(union_type_or_literal_variant_type
          <(Option<ParsedSubExpr>,
            (Option<(Label, ParsedSubExpr)>,
             DupTreeMap<Label, Option<ParsedSubExpr>>))>;
                children!(
        [expression(e), non_empty_union_type_or_literal(rest)] => {
            (Some(e), rest)
        },
        [expression(e)] => {
            (Some(e), (None, Default::default()))
        },
        [non_empty_union_type_or_literal(rest)] => {
            (None, rest)
        },
        [] => {
            (None, (None, Default::default()))
        },
    ));

    rule!(non_empty_list_literal<ParsedSubExpr> as expression; span;
          children!(
        [expression(items)..] => spanned(
            span,
            NEListLit(items.collect())
        )
    ));

    rule!(final_expression<ParsedSubExpr> as expression; children!(
        [expression(e), EOI(_eoi)] => e
    ));
}

pub fn parse_expr(s: &str) -> ParseResult<ParsedSubExpr> {
    let mut pairs = DhallParser::parse(Rule::final_expression, s)?;
    let rc_input = s.to_string().into();
    let expr = do_parse(rc_input, pairs.next().unwrap())?;
    assert_eq!(pairs.next(), None);
    match expr {
        ParsedValue::expression(e) => Ok(e),
        _ => unreachable!(),
    }
    // Ok(BoolLit(false))
}

#[test]
fn test_parse() {
    // let expr = r#"{ x = "foo", y = 4 }.x"#;
    // let expr = r#"(1 + 2) * 3"#;
    let expr = r#"(1) + 3 * 5"#;
    println!("{:?}", parse_expr(expr));
    match parse_expr(expr) {
        Err(e) => {
            println!("{:?}", e);
            println!("{}", e);
        }
        ok => println!("{:?}", ok),
    };
    // assert!(false);
}
