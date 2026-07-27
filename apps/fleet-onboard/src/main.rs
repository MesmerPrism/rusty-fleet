use std::env;
use std::path::PathBuf;

use fleet_onboarding::{Command, execute};

fn usage() -> &'static str {
    "usage:
  fleet-onboard validate-tool --request <private-request.json>
  fleet-onboard plan --request <private-request.json>
  fleet-onboard apply --request <private-request.json> --confirm-plan-sha256 <digest> --non-interactive
  fleet-onboard cleanup-plan --inventory <private-inventory.json>
  fleet-onboard cleanup-apply --inventory <private-inventory.json> --confirm-cleanup-sha256 <digest>
  fleet-onboard revoke-plan --inventory <private-inventory.json>"
}

fn parse() -> Result<Command, String> {
    let mut args = env::args_os();
    let _exe = args.next();
    let verb = args
        .next()
        .and_then(|v| v.into_string().ok())
        .ok_or_else(|| usage().to_owned())?;
    let rest: Vec<_> = args.collect();
    let text = |index: usize| {
        rest.get(index)
            .and_then(|v| v.clone().into_string().ok())
            .ok_or_else(|| usage().to_owned())
    };
    let path = |index: usize| text(index).map(PathBuf::from);
    match (verb.as_str(), rest.len()) {
        ("validate-tool", 2) if rest[0] == "--request" => {
            Ok(Command::ValidateTool { request: path(1)? })
        }
        ("plan", 2) if rest[0] == "--request" => Ok(Command::Plan { request: path(1)? }),
        ("apply", 5)
            if rest[0] == "--request"
                && rest[2] == "--confirm-plan-sha256"
                && rest[4] == "--non-interactive" =>
        {
            Ok(Command::Apply {
                request: path(1)?,
                confirmation: text(3)?,
            })
        }
        ("cleanup-plan", 2) if rest[0] == "--inventory" => Ok(Command::CleanupPlan {
            inventory: path(1)?,
        }),
        ("cleanup-apply", 4)
            if rest[0] == "--inventory" && rest[2] == "--confirm-cleanup-sha256" =>
        {
            Ok(Command::CleanupApply {
                inventory: path(1)?,
                confirmation: text(3)?,
            })
        }
        ("revoke-plan", 2) if rest[0] == "--inventory" => Ok(Command::RevokePlan {
            inventory: path(1)?,
        }),
        _ => Err(usage().to_owned()),
    }
}

fn main() {
    let result = parse().and_then(execute);
    match result {
        Ok(value) => match serde_json::to_string_pretty(&value) {
            Ok(json) => println!("{json}"),
            Err(_) => {
                eprintln!("fleet-onboard: output_serialization_failed");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("fleet-onboard: {error}");
            std::process::exit(1);
        }
    }
}
