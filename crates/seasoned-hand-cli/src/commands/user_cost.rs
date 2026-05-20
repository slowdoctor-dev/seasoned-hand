use anyhow::{Result, anyhow};
use clap::Subcommand;

use crate::client::{ApiClient, into_anyhow};

#[derive(Debug, Subcommand)]
pub enum UserCostCmd {
    /// Manually run user_cost ledger reconciliation for a month.
    Reconcile {
        /// Month in YYYY-MM format.
        #[arg(long = "month")]
        month: String,
    },
}

pub async fn run(cmd: UserCostCmd, client: &ApiClient, json: bool) -> Result<()> {
    match cmd {
        UserCostCmd::Reconcile { month } => {
            let month_yyyymm = parse_month_to_yyyymm(&month)?;
            let report = into_anyhow(client.user_cost_reconcile(&month_yyyymm).await)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!(
                    "rows_checked={} drifted_rows={}",
                    report.rows_checked, report.drifted_rows
                );
                if report.drifts.is_empty() {
                    println!("(no drift findings)");
                } else {
                    for d in report.drifts {
                        println!(
                            "{} {} {} expected={} observed={} delta_pct={:.6}",
                            d.tenant_id,
                            d.user_id,
                            d.month_yyyymm,
                            d.expected_cost_cents,
                            d.observed_cost_cents,
                            d.delta_pct
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn parse_month_to_yyyymm(value: &str) -> Result<String> {
    let (year, month) = value
        .split_once('-')
        .ok_or_else(|| anyhow!("month must be YYYY-MM"))?;
    if year.len() != 4 || month.len() != 2 {
        return Err(anyhow!("month must be YYYY-MM"));
    }
    let y: u32 = year.parse()?;
    let m: u32 = month.parse()?;
    if y < 1970 || !(1..=12).contains(&m) {
        return Err(anyhow!("month must be YYYY-MM"));
    }
    Ok(format!("{year}{month}"))
}
