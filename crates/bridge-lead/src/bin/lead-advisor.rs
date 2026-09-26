//! `lead-advisor`: a small CLI around [`bridge_lead::advise`] (`docs/design/14-lead.md` §3,
//! roadmap phase 6.2). Only built with the `dds` feature (it needs a real
//! [`bridge::dd::DoubleDummy`] solver and PBN reading).
//!
//! ```text
//! lead-advisor <FILE.pbn> [--board N] [--samples N] [--seed N] [--system PATH] [--uniform]
//! lead-advisor --hand <HAND> --auction "<calls...>" --dealer <SEAT> --vul <VUL>
//!              [--samples N] [--seed N] [--system PATH] [--uniform]
//! ```
//!
//! `--system` defaults to `systems/sayc/sayc.bml` (relative to the current directory, i.e. run from
//! the workspace root, matching `systems/README.md`'s own examples); `--uniform` swaps
//! [`bridge_sample::ConstraintProposal`] (the default) for [`bridge_sample::UniformProposal`].
//! Both proposals still need a compiled system, because [`bridge_bidding::interpret`] and the
//! bidding-likelihood importance weights read it regardless of which proposal draws the deals.

use std::process::ExitCode;
use std::sync::Arc;

use bridge::system::lexer::FsLoader;
use bridge::system::{CompileOptions, NaturalInference};
use bridge_bidding::Table;
use bridge_core::{Auction, Call, Hand, Seat, Vulnerability};
use bridge_format::pbn;
use bridge_lead::{LeadAdvice, LeadError, LeadOptions, LeadQuery};
use bridge_sample::{ConstraintProposal, Proposal, UniformProposal};

const DEFAULT_SYSTEM: &str = "systems/sayc/sayc.bml";

struct Args {
    pbn_file: Option<String>,
    hand: Option<String>,
    auction: Option<String>,
    dealer: Seat,
    vulnerability: Vulnerability,
    board: Option<u16>,
    samples: usize,
    seed: u64,
    system: String,
    uniform: bool,
}

impl Default for Args {
    fn default() -> Args {
        Args {
            pbn_file: None,
            hand: None,
            auction: None,
            dealer: Seat::North,
            vulnerability: Vulnerability::None,
            board: None,
            samples: LeadOptions::default().samples,
            seed: 0,
            system: DEFAULT_SYSTEM.to_string(),
            uniform: false,
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut positional = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--hand" => args.hand = Some(value("--hand")?),
            "--auction" => args.auction = Some(value("--auction")?),
            "--dealer" => {
                let s = value("--dealer")?;
                args.dealer = s.parse().map_err(|e| format!("--dealer {s:?}: {e}"))?;
            }
            "--vul" => {
                let s = value("--vul")?;
                args.vulnerability = s.parse().map_err(|e| format!("--vul {s:?}: {e}"))?;
            }
            "--board" => {
                let s = value("--board")?;
                args.board = Some(s.parse().map_err(|e| format!("--board {s:?}: {e}"))?);
            }
            "--samples" => {
                let s = value("--samples")?;
                args.samples = s.parse().map_err(|e| format!("--samples {s:?}: {e}"))?;
            }
            "--seed" => {
                let s = value("--seed")?;
                args.seed = s.parse().map_err(|e| format!("--seed {s:?}: {e}"))?;
            }
            "--system" => args.system = value("--system")?,
            "--uniform" => args.uniform = true,
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other:?}"));
            }
            other => positional.push(other.to_string()),
        }
    }
    if positional.len() > 1 {
        return Err(format!("expected at most one PBN file, got {positional:?}"));
    }
    args.pbn_file = positional.into_iter().next();
    Ok(args)
}

fn load_table(system_path: &str) -> Result<Table, String> {
    let source =
        std::fs::read_to_string(system_path).map_err(|e| format!("reading {system_path}: {e}"))?;
    let (ir, lints) =
        bridge::system::compile(system_path, &source, &FsLoader, &CompileOptions::default());
    let errors: Vec<_> = lints
        .iter()
        .filter(|l| l.severity == bridge::system::Severity::Error)
        .collect();
    if !errors.is_empty() {
        return Err(format!(
            "{system_path}: {} error-level lint(s), e.g. {:?}",
            errors.len(),
            errors[0]
        ));
    }
    Ok(Table::uniform(
        Arc::new(ir),
        Arc::new(NaturalInference::default()),
    ))
}

/// One query built either from a PBN game or from `--hand`/`--auction`.
struct Query {
    label: String,
    auction: Auction,
    leader_hand: Hand,
}

fn queries_from_pbn(path: &str, board: Option<u16>) -> Result<Vec<Query>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading {path}: {e}"))?;
    let (file, _warnings) = pbn::parse_lenient(&bytes);
    let mut queries = Vec::new();
    let mut previous = None;
    for game in &file.games {
        let view = game.view(previous.as_ref());
        let view = match view {
            Ok(v) => v,
            Err(_) => continue,
        };
        let keep = view
            .board
            .zip(board)
            .map(|(b, want)| b == want)
            .unwrap_or(board.is_none());
        if keep {
            // Deliberately nested `if let`s, not a let-chain: MSRV 1.85 predates them
            // (`docs/design/01-workspace.md`).
            if let (Some(auction), Some(contract), Some(partial)) =
                (&view.auction, view.contract, view.deal)
            {
                if auction.is_complete() && !auction.is_passed_out() {
                    if let Some(deal) = partial.complete() {
                        queries.push(Query {
                            label: view
                                .board
                                .map(|b| format!("board {b}"))
                                .unwrap_or_else(|| "(no board number)".to_string()),
                            auction: auction.clone(),
                            leader_hand: deal.hand(contract.leader()),
                        });
                    }
                }
            }
        }
        previous = Some(view);
    }
    Ok(queries)
}

fn query_from_args(args: &Args) -> Result<Query, String> {
    let hand_str = args
        .hand
        .as_deref()
        .ok_or_else(|| "either a PBN file or --hand/--auction is required".to_string())?;
    let auction_str = args
        .auction
        .as_deref()
        .ok_or_else(|| "--auction is required when --hand is given".to_string())?;
    let leader_hand: Hand = hand_str
        .parse()
        .map_err(|e| format!("--hand {hand_str:?}: {e}"))?;
    let mut auction = Auction::new(args.dealer, args.vulnerability);
    for token in auction_str.split_whitespace() {
        let call: Call = token
            .parse()
            .map_err(|e| format!("--auction: call {token:?}: {e}"))?;
        auction.push(call).map_err(|e| format!("--auction: {e}"))?;
    }
    Ok(Query {
        label: "command line".to_string(),
        auction,
        leader_hand,
    })
}

fn print_advice(label: &str, advice: &LeadAdvice) {
    println!(
        "{label}: {} by {:?}, leader {:?} (samples {}, ESS {:.1})",
        advice.contract.bid, advice.declarer, advice.leader, advice.samples_used, advice.ess
    );
    for lead in &advice.leads {
        let equivalents = if lead.equivalents.is_empty() {
            String::new()
        } else {
            format!(
                " (= {})",
                lead.equivalents
                    .iter()
                    .map(|c| format!("{c:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!(
            "  {}. {:?}{equivalents}: {:.2} tricks (± {:.2}), P(set) = {:.2}",
            lead.rank, lead.card, lead.mean_defence_tricks, lead.std_error, lead.set_probability
        );
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let table = load_table(&args.system)?;

    let queries = match &args.pbn_file {
        Some(path) => queries_from_pbn(path, args.board)?,
        None => vec![query_from_args(&args)?],
    };
    if queries.is_empty() {
        return Err("no game with a complete, non-passed-out auction and a full deal found".into());
    }

    let opts = LeadOptions {
        samples: args.samples,
        seed: args.seed,
        ..LeadOptions::default()
    };

    let uniform = UniformProposal;
    let constraint = ConstraintProposal::default();
    let proposal: &dyn Proposal = if args.uniform { &uniform } else { &constraint };

    let Some(dd) = bridge::dd::dds() else {
        return Err(
            "no DDS solver in this build (run `cargo xtask dds vendor` and rebuild)".to_string(),
        );
    };

    for query in &queries {
        let lead_query = LeadQuery {
            auction: &query.auction,
            leader_hand: query.leader_hand,
        };
        match bridge_lead::advise(&table, &lead_query, proposal, dd.as_ref(), &opts) {
            Ok(advice) => print_advice(&query.label, &advice),
            Err(LeadError::PassedOut | LeadError::IncompleteAuction) => {
                // Already filtered out for the PBN path; only reachable for `--hand`/`--auction`.
                eprintln!("{}: auction has no opening lead", query.label);
            }
            Err(e) => eprintln!("{}: {e}", query.label),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("lead-advisor: {message}");
            ExitCode::FAILURE
        }
    }
}
