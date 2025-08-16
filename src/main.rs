#![feature(test)]

use crate::deviceinfo::DeviceInfo;
use crate::mapping::*;
use crate::remapper::*;
use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueHint};
use std::path::PathBuf;
use std::time::Duration;

mod deviceinfo;
mod mapping;
mod remapper;

#[derive(Debug, Parser)]
#[command(
    name = "evremap",
    version,
    about = "Remap Linux input (evdev) events via a simple TOML config"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Command>,

    #[arg(value_name = "CONFIG-FILE", value_hint = ValueHint::FilePath)]
    config_file: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    ListDevices,

    ListKeys,

    DebugEvents {
        #[arg(long)]
        device_name: String,

        #[arg(long)]
        phys: Option<String>,
    },

    #[command(
        arg_required_else_help = true,
        about = "Apply mappings from a TOML config to a device"
    )]
    Remap {
        #[arg(
            value_name = "/path/to/config.toml",
            value_hint = ValueHint::FilePath,
            help = "Path to the remapping config (TOML). Required."
        )]
        config_file: PathBuf,

        #[arg(short, long, default_value_t = 2.0)]
        delay: f64,

        #[arg(long)]
        device_name: Option<String>,

        #[arg(long)]
        phys: Option<String>,

        #[arg(long)]
        wait_for_device: bool,
    },
}

pub fn list_keys() -> Result<()> {
    let mut keys: Vec<String> = EventCode::EV_KEY(KeyCode::KEY_RESERVED)
        .iter()
        .filter_map(|code| match code {
            EventCode::EV_KEY(_) => Some(format!("{code}")),
            _ => None,
        })
        .collect();
    keys.sort();

    Ok(())
}

fn setup_logger() {
    let mut builder = env_logger::Builder::new();
    builder.filter_level(log::LevelFilter::Info);
    let env = env_logger::Env::new()
        .filter("EVREMAP_LOG")
        .write_style("EVREMAP_LOG_STYLE");
    builder.parse_env(env);
    builder.init();
}

fn get_device(
    device_name: &str,
    phys: Option<&str>,
    wait_for_device: bool,
) -> anyhow::Result<DeviceInfo> {
    match deviceinfo::DeviceInfo::with_name(device_name, phys) {
        Ok(dev) => return Ok(dev),
        Err(err) if !wait_for_device => return Err(err),
        Err(err) => {
            log::warn!("{err:#}. Will wait until it is attached.");
        },
    }

    const MAX_SLEEP: Duration = Duration::from_secs(10);
    const ONE_SECOND: Duration = Duration::from_secs(1);
    let mut sleep = ONE_SECOND;

    loop {
        std::thread::sleep(sleep);
        sleep = (sleep + ONE_SECOND).min(MAX_SLEEP);

        match deviceinfo::DeviceInfo::with_name(device_name, phys) {
            Ok(dev) => return Ok(dev),
            Err(err) => {
                log::debug!("{err:#}");
            },
        }
    }
}

fn debug_events(device: DeviceInfo) -> Result<()> {
    let f =
        std::fs::File::open(&device.path).context(format!("opening {}", device.path.display()))?;
    let input = evdev_rs::Device::new_from_file(f).with_context(|| {
        format!("failed to create new Device from file {}", device.path.display())
    })?;

    loop {
        let (status, event) =
            input.next_event(evdev_rs::ReadFlag::NORMAL | evdev_rs::ReadFlag::BLOCKING)?;
        match status {
            evdev_rs::ReadStatus::Success => {
                if let EventCode::EV_KEY(key) = event.event_code {
                    log::info!("{key:?} {}", event.value);
                }
            },
            evdev_rs::ReadStatus::Sync => anyhow::bail!("ReadStatus::Sync!"),
        }
    }
}

fn do_remap(
    config_file: PathBuf,
    delay: f64,
    device_name: Option<String>,
    phys: Option<String>,
    wait_for_device: bool,
) -> Result<()> {
    let mut mapping_config = MappingConfig::from_file(&config_file)
        .context(format!("loading MappingConfig from {}", config_file.display()))?;

    if let Some(device) = device_name {
        mapping_config.device_name = Some(device);
    }
    if let Some(phys) = phys {
        mapping_config.phys = Some(phys);
    }

    let device_name = mapping_config
        .device_name
        .as_deref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "device_name is missing; specify it either in the config file or via the \
                 --device-name command line option"
            )
        })?;

    log::warn!("Short delay: release any keys now!");
    std::thread::sleep(Duration::from_secs_f64(delay));

    let device_info = get_device(device_name, mapping_config.phys.as_deref(), wait_for_device)?;

    let mut mapper = InputMapper::create_mapper(device_info.path, mapping_config.mappings)?;
    mapper.run_mapper()
}

fn main() -> Result<()> {
    setup_logger();
    let cli = Cli::parse();

    match cli.cmd {
        Some(Command::ListDevices) => deviceinfo::list_devices(),
        Some(Command::ListKeys) => list_keys(),
        Some(Command::DebugEvents { device_name, phys }) => {
            let device_info = get_device(&device_name, phys.as_deref(), false)?;
            debug_events(device_info)
        },
        Some(Command::Remap { config_file, delay, device_name, phys, wait_for_device }) => {
            do_remap(config_file, delay, device_name, phys, wait_for_device)
        },
        None => {
            if let Some(config_file) = cli.config_file {
                do_remap(config_file, 2.0, None, None, false)
            } else {
                Cli::command().print_help()?;
                println!();
                Ok(())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_default_cmd() {
        let cli = Cli::try_parse_from(["evremap", "foo.toml"]).expect("parse ok");
        assert!(cli.cmd.is_none());
        assert_eq!(cli.config_file, Some(PathBuf::from("foo.toml")));
    }

    #[test]
    fn parse_remap_cmd() {
        let cli = Cli::try_parse_from([
            "evremap",
            "remap",
            "foo.toml",
            "--delay",
            "1.5",
            "--device-name",
            "dev",
            "--phys",
            "p",
            "--wait-for-device",
        ])
        .expect("parse ok");

        let Some(Command::Remap { config_file, delay, device_name, phys, wait_for_device }) =
            cli.cmd
        else {
            panic!("expected 'remap' subcommand");
        };

        assert_eq!(config_file, PathBuf::from("foo.toml"));
        assert!((delay - 1.5).abs() < f64::EPSILON);
        assert_eq!(device_name.as_deref(), Some("dev"));
        assert_eq!(phys.as_deref(), Some("p"));
        assert!(wait_for_device);
    }
}
