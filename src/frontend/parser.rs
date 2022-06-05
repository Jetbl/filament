#![allow(clippy::upper_case_acronyms)]

//! Parser for Calyx programs.
use super::IntervalTime;
use crate::core::{self, Id};
use crate::errors::{self, FilamentResult};
use pest_consume::{match_nodes, Error, Parser};
use std::fs;
use std::path::Path;
use std::rc::Rc;

/// Data associated with parsing the file.
#[derive(Clone)]
struct UserData {
    /// Input to the parser
    pub input: Rc<str>,
    /// Path of the file
    pub file: Rc<str>,
}

type ParseResult<T> = Result<T, Error<Rule>>;
// user data is the input program so that we can create ir::Id's
// that have a reference to the input string
type Node<'i> = pest_consume::Node<'i, Rule, UserData>;

type Ports = Vec<core::PortDef<IntervalTime>>;

// include the grammar file so that Cargo knows to rebuild this file on grammar changes
const _GRAMMAR: &str = include_str!("syntax.pest");

pub enum ExtOrComp {
    Ext(core::Signature<IntervalTime>),
    Comp(core::Component<IntervalTime>),
}

pub enum PdOrInt {
    Pd(core::PortDef<IntervalTime>),
    Int((core::Id, core::Id)),
}

#[derive(Parser)]
#[grammar = "frontend/syntax.pest"]
pub struct FilamentParser;

impl FilamentParser {
    /// Parse a Calyx program into an AST representation.
    pub fn parse_file(
        path: &Path,
    ) -> FilamentResult<core::Namespace<IntervalTime>> {
        let content = &fs::read(path).map_err(|err| {
            errors::Error::invalid_file(format!(
                "Failed to read {}: {}",
                path.to_string_lossy(),
                err
            ))
        })?;
        let string_content = std::str::from_utf8(content)?;

        let user_data = UserData {
            input: Rc::from(string_content),
            file: Rc::from(path.to_string_lossy()),
        };
        let inputs = FilamentParser::parse_with_userdata(
            Rule::file,
            string_content,
            user_data,
        )
        .map_err(|e| e.with_path(&path.to_string_lossy()))?;
        let input = inputs.single()?;
        Ok(FilamentParser::file(input)?)
    }

    fn get_span(node: &Node) -> errors::Span {
        let ud = node.user_data();
        errors::Span::new(
            node.as_span(),
            Rc::clone(&ud.file),
            Rc::clone(&ud.input),
        )
    }
}

#[pest_consume::parser]
impl FilamentParser {
    fn EOI(_input: Node) -> ParseResult<()> {
        Ok(())
    }

    // ================ Literals =====================
    fn identifier(input: Node) -> ParseResult<Id> {
        Ok(Id::from(input.as_str()))
    }

    fn char(input: Node) -> ParseResult<&str> {
        Ok(input.as_str())
    }

    fn string_lit(input: Node) -> ParseResult<String> {
        Ok(match_nodes!(
            input.into_children();
            [char(c)..] => c.collect::<Vec<_>>().join("")
        ))
    }

    fn bitwidth(input: Node) -> ParseResult<u64> {
        input
            .as_str()
            .parse::<u64>()
            .map_err(|_| input.error("Expected valid bitwidth"))
    }

    // ================ Intervals =====================
    fn time_base(input: Node) -> ParseResult<IntervalTime> {
        Ok(match_nodes!(
            input.into_children();
            [identifier(var)] => IntervalTime::Abstract(var),
            [time(l), time(r)] => IntervalTime::binop_max(l, r),
            [bitwidth(time)] => IntervalTime::Concrete(time),
        ))
    }
    fn time_expr(input: Node) -> ParseResult<IntervalTime> {
        Ok(match_nodes!(
            input.into_children();
            [time_base(l), time(r)] => IntervalTime::binop_add(l, r),
        ))
    }

    fn time(input: Node) -> ParseResult<IntervalTime> {
        Ok(match_nodes!(
            input.into_children();
            [time_expr(time)] => time,
            [time_base(time)] => time
        ))
    }

    fn interval_range(input: Node) -> ParseResult<core::Range<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [time(start), time(end)] => core::Range { start, end }
        ))
    }

    // ================ Signature =====================

    fn interface(input: Node) -> ParseResult<core::Id> {
        Ok(match_nodes!(
            input.into_children();
            [identifier(tvar)] => tvar,
        ))
    }

    fn port_def(input: Node) -> ParseResult<PdOrInt> {
        let pd = match_nodes!(
            input.clone().into_children();
            [interface(time_var), identifier(name), bitwidth(_)] => {
                Ok(PdOrInt::Int((name, time_var)))
            },
            [interval_range(range), identifier(name), bitwidth(bitwidth)] => {
                core::PortDef::new(name, core::Interval::new(range), bitwidth).map(PdOrInt::Pd)
            },
            [interval_range(range), interval_range(exact), identifier(name), bitwidth(bitwidth)] => {
                core::PortDef::new(name, core::Interval::new(range).with_exact(exact), bitwidth).map(PdOrInt::Pd)
            }
        );
        pd.map_err(|err| input.error(format!("{err:?}")))
    }

    fn abstract_var(input: Node) -> ParseResult<Vec<Id>> {
        Ok(match_nodes!(
            input.into_children();
            [identifier(vars)..] => vars.collect(),
        ))
    }

    fn ports(
        input: Node,
    ) -> ParseResult<(
        Vec<core::PortDef<IntervalTime>>,
        Vec<(core::Id, core::Id)>,
    )> {
        Ok(match_nodes!(
            input.into_children();
            [port_def(ins)..] => {
                let mut interface_signals = vec![];
                let mut ports = vec![];
                for m in ins {
                    match m {
                        PdOrInt::Pd(port) => ports.push(port),
                        PdOrInt::Int(int) => interface_signals.push(int),
                    }
                }
                (ports, interface_signals)
            }
        ))
    }

    fn arrow(input: Node) -> ParseResult<()> {
        Ok(())
    }

    fn io(
        input: Node,
    ) -> ParseResult<(Ports, Ports, Vec<(core::Id, core::Id)>)> {
        match_nodes!(
            input.clone().into_children();
            [arrow(_)] => Ok((vec![], vec![], vec![])),
            [ports((ins, interface)), arrow(_)] =>  Ok((ins, vec![], interface)),
            [arrow(_), ports((outs, out_interface))] =>  {
                if !out_interface.is_empty() {
                    Err(input.error("Output interface ports not supported"))
                } else {
                    Ok((vec![], outs, vec![]))
                }
            },
            [ports((ins, interface)), arrow(_), ports((outs, out_interface))] => {
                if !out_interface.is_empty() {
                    Err(input.error("Output interface ports not supported"))
                } else {
                    Ok((ins, outs, interface))
                }
            }
        )
    }

    // ================ Cells =====================
    fn instance(input: Node) -> ParseResult<core::Instance> {
        Ok(match_nodes!(
            input.into_children();
            [identifier(name), identifier(component)] => core::Instance {
                name, component
            }
        ))
    }

    // ================ Assignments =====================
    fn port(input: Node) -> ParseResult<core::Port> {
        Ok(match_nodes!(
            input.into_children();
            [bitwidth(constant)] => core::Port::Constant(constant),
            [identifier(name)] => core::Port::ThisPort(name),
            [identifier(comp), identifier(name)] => core::Port::CompPort {
                comp, name
            }
        ))
    }

    fn arguments(input: Node) -> ParseResult<Vec<core::Port>> {
        Ok(match_nodes!(
            input.into_children();
            [] => vec![],
            [port(ports)..] => ports.collect(),
        ))
    }

    fn time_args(input: Node) -> ParseResult<Vec<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [time(args)..] => args.collect(),
        ))
    }

    fn invocation(input: Node) -> ParseResult<core::Invoke<IntervalTime>> {
        let span = Self::get_span(&input);
        Ok(match_nodes!(
            input.into_children();
            [
                identifier(bind),
                identifier(comp),
                time_args(abstract_vars),
                arguments(ports)
            ] => core::Invoke::new(bind, comp, abstract_vars, Some(ports)).with_span(Some(span)),
            [
                identifier(bind),
                identifier(comp),
                time_args(abstract_vars),
            ] => core::Invoke::new(bind, comp, abstract_vars, None).with_span(Some(span))
        ))
    }
    fn gte(input: Node) -> ParseResult<()> {
        Ok(())
    }
    fn lte(input: Node) -> ParseResult<()> {
        Ok(())
    }

    fn gt(input: Node) -> ParseResult<()> {
        Ok(())
    }
    fn lt(input: Node) -> ParseResult<()> {
        Ok(())
    }
    fn eq(input: Node) -> ParseResult<()> {
        Ok(())
    }
    fn order_op(input: Node) -> ParseResult<core::OrderOp> {
        Ok(match_nodes!(
            input.into_children();
            [gte(_)] => core::OrderOp::Gte,
            [lte(_)] => core::OrderOp::Lte,
            [gt(_)] => core::OrderOp::Gt,
            [lt(_)] => core::OrderOp::Lt,
            [eq(_)] => core::OrderOp::Eq,
        ))
    }
    fn constraint(input: Node) -> ParseResult<core::Constraint<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [
                time(left),
                order_op(op),
                time(right)
            ] => core::Constraint {
                left, right, op
            }
        ))
    }
    fn constraints(
        input: Node,
    ) -> ParseResult<Vec<core::Constraint<IntervalTime>>> {
        Ok(match_nodes!(
            input.into_children();
            [] => Vec::default(),
            [constraint(cons)..] => cons.collect()
        ))
    }

    // ================ Component =====================
    fn signature(input: Node) -> ParseResult<core::Signature<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [
                identifier(name),
                abstract_var(abstract_vars),
                io(io),
                constraints(constraints)
            ] => {
                let (inputs, outputs, interface_signals) = io;
                core::Signature {
                    name,
                    abstract_vars,
                    interface_signals: interface_signals.into_iter().collect(),
                    inputs,
                    outputs,
                    constraints,
                }
            },
            [
                identifier(name),
                io(io),
                constraints(constraints)
            ] => {
                let (inputs, outputs, interface_signals) = io;
                core::Signature {
                    name,
                    abstract_vars: vec![],
                    interface_signals: interface_signals.into_iter().collect(),
                    inputs,
                    outputs,
                    constraints
                }
            }
        ))
    }

    fn when(input: Node) -> ParseResult<core::When<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [time(time), command(body)..] => core::When {
                time, commands: body.collect()
            }
        ))
    }

    fn guard(input: Node) -> ParseResult<core::Guard> {
        Ok(match_nodes!(
            input.into_children();
            [port(p)] => core::Guard::Port(p),
            [port(p), guard(g)] => {
                core::Guard::Or(Box::new(core::Guard::Port(p)), Box::new(g))
            }
        ))
    }

    fn connect(input: Node) -> ParseResult<core::Connect> {
        let span = Self::get_span(&input);
        Ok(match_nodes!(
            input.into_children();
            [port(dst), port(src)] => core::Connect::new(dst, src, None).with_span(span),
            [port(dst), guard(guard), port(src)] => {
                core::Connect::new(dst, src, Some(guard)).with_span(span)
            }
        ))
    }

    fn command(input: Node) -> ParseResult<core::Command<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [invocation(assign)] => core::Command::Invoke(assign),
            [instance(cell)] => core::Command::Instance(cell),
            [when(wh)] => core::Command::When(wh),
            [connect(con)] => core::Command::Connect(con),
        ))
    }

    fn component(input: Node) -> ParseResult<core::Component<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [
                signature(sig),
                command(body)..
            ] => {
                core::Component::new(sig, body.collect())
            }
        ))
    }

    fn external(input: Node) -> ParseResult<core::Signature<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [signature(sig)] => sig,
        ))
    }

    fn comp_or_ext(input: Node) -> ParseResult<ExtOrComp> {
        Ok(match_nodes!(
            input.into_children();
            [external(sig)] => ExtOrComp::Ext(sig),
            [component(comp)] => ExtOrComp::Comp(comp),
        ))
    }

    fn imports(input: Node) -> ParseResult<Vec<String>> {
        Ok(match_nodes!(
            input.into_children();
            [string_lit(path)..] => path.collect()
        ))
    }

    fn file(input: Node) -> ParseResult<core::Namespace<IntervalTime>> {
        Ok(match_nodes!(
            input.into_children();
            [imports(imps), comp_or_ext(mixed).., _EOI] => {
                let mut namespace = core::Namespace {
                    imports: imps,
                    signatures: vec![],
                    components: vec![],
                };
                for m in mixed {
                    match m {
                        ExtOrComp::Ext(sig) => namespace.signatures.push(sig),
                        ExtOrComp::Comp(comp) => namespace.components.push(comp),
                    }
                }
                namespace
            }
        ))
    }
}
