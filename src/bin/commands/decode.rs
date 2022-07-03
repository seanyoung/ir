use super::{find_devices, Purpose};
use cir::{
    lirc,
    lircd_conf::{parse, Remote},
    log::Log,
};
use irp::{mode2, rawir, InfraredData, Irp, Matcher, NFA};
use itertools::Itertools;
use num_integer::Integer;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn decode(matches: &clap::ArgMatches, log: &Log) {
    let remotes;
    let graphviz = matches.is_present("GRAPHVIZ");

    let irps = if let Some(i) = matches.value_of("IRP") {
        let irp = match Irp::parse(i) {
            Ok(m) => m,
            Err(s) => {
                eprintln!("unable to parse irp ‘{}’: {}", i, s);
                std::process::exit(2);
            }
        };

        let nfa = irp.build_nfa().unwrap();

        if graphviz {
            let filename = "irp_nfa.dot";
            log.info(&format!("saving nfa as {}", filename));

            nfa.dotgraphviz(filename);
        }

        vec![(None, nfa)]
    } else if let Some(filename) = matches.value_of_os("LIRCDCONF") {
        remotes = match parse(filename, log) {
            Ok(r) => r,
            Err(_) => std::process::exit(2),
        };

        remotes
            .iter()
            .map(|remote| {
                let irp = remote.irp();

                log.info(&format!("found remote {}", remote.name));
                log.info(&format!("IRP {}", irp));

                let irp = Irp::parse(&irp).unwrap();

                let nfa = irp.build_nfa().unwrap();

                if graphviz {
                    let filename = format!("{}_nfa.dot", remote.name);
                    log.info(&format!("saving nfa as {}", filename));

                    nfa.dotgraphviz(&filename);
                }

                (Some(remote), nfa)
            })
            .collect()
    } else {
        unreachable!();
    };

    let mut input_on_cli = false;

    if let Some(files) = matches.values_of_os("FILE") {
        input_on_cli = true;

        for filename in files {
            let input = match fs::read_to_string(filename) {
                Ok(s) => s,
                Err(s) => {
                    log.error(&format!("{}: {}", Path::new(filename).display(), s));
                    std::process::exit(2);
                }
            };

            log.info(&format!(
                "parsing ‘{}’ as rawir",
                filename.to_string_lossy()
            ));

            match rawir::parse(&input) {
                Ok(raw) => {
                    log.info(&format!("decoding: {}", rawir::print_to_string(&raw)));
                    process(&raw, &irps, matches, log);
                }
                Err(msg) => {
                    log.info(&format!(
                        "parsing ‘{}’ as mode2",
                        filename.to_string_lossy()
                    ));

                    match mode2::parse(&input) {
                        Ok(m) => {
                            log.info(&format!("decoding: {}", rawir::print_to_string(&m.raw)));
                            process(&m.raw, &irps, matches, log);
                        }
                        Err((line_no, error)) => {
                            log.error(&format!(
                                "{}: parse as rawir: {}",
                                Path::new(filename).display(),
                                msg
                            ));
                            log.error(&format!(
                                "{}:{}: parse as mode2: {}",
                                Path::new(filename).display(),
                                line_no,
                                error
                            ));
                            std::process::exit(2);
                        }
                    }
                }
            }
        }
    }

    if let Some(rawirs) = matches.values_of("RAWIR") {
        input_on_cli = true;

        for rawir in rawirs {
            match rawir::parse(rawir) {
                Ok(raw) => {
                    log.info(&format!("decoding: {}", rawir::print_to_string(&raw)));
                    process(&raw, &irps, matches, log);
                }
                Err(msg) => {
                    log.error(&format!("parsing ‘{}’: {}", rawir, msg));
                    std::process::exit(2);
                }
            }
        }
    }

    if !input_on_cli {
        // open lirc
        let rcdev = find_devices(matches, Purpose::Receive);

        if let Some(lircdev) = rcdev.lircdev {
            let lircpath = PathBuf::from(lircdev);

            let mut lircdev = match lirc::open(&lircpath) {
                Ok(l) => l,
                Err(s) => {
                    eprintln!("error: {}: {}", lircpath.display(), s);
                    std::process::exit(1);
                }
            };

            if matches.is_present("LEARNING") {
                let mut learning_mode = false;

                if lircdev.can_measure_carrier() {
                    if let Err(err) = lircdev.set_measure_carrier(true) {
                        eprintln!(
                            "error: {}: failed to enable measure carrier: {}",
                            lircdev, err
                        );
                        std::process::exit(1);
                    }
                    learning_mode = true;
                }

                if lircdev.can_use_wideband_receiver() {
                    if let Err(err) = lircdev.set_wideband_receiver(true) {
                        eprintln!(
                            "error: {}: failed to enable wideband receiver: {}",
                            lircdev, err
                        );
                        std::process::exit(1);
                    }
                    learning_mode = true;
                }

                if !learning_mode {
                    eprintln!(
                        "error: {}: lirc device does not support learning mode",
                        lircdev
                    );
                    std::process::exit(1);
                }
            }

            if lircdev.can_receive_raw() {
                let mut rawbuf = Vec::with_capacity(1024);
                let resolution = lircdev.receiver_resolution().unwrap_or(100);

                let mut matchers = irps
                    .iter()
                    .map(|(remote, nfa)| (remote, nfa.matcher(resolution, 100)))
                    .collect::<Vec<(&Option<&Remote>, Matcher)>>();

                loop {
                    if let Err(err) = lircdev.receive_raw(&mut rawbuf) {
                        eprintln!("error: {}", err);
                        std::process::exit(1);
                    }

                    // TODO: print rawbuf to log.info()

                    for raw in &rawbuf {
                        let ir = if raw.is_pulse() {
                            InfraredData::Flash(raw.value())
                        } else if raw.is_space() || raw.is_timeout() {
                            InfraredData::Gap(raw.value())
                        } else if raw.is_overflow() {
                            InfraredData::Reset
                        } else {
                            continue;
                        };

                        for (remote, matcher) in &mut matchers {
                            if let Some(var) = matcher.input(ir) {
                                if let Some(remote) = remote {
                                    // lirc
                                    let decoded_code = var["CODE"] as u64;

                                    // TODO: raw codes
                                    if let Some(code) = remote
                                        .codes
                                        .iter()
                                        .find(|code| code.code[0] == decoded_code)
                                    {
                                        println!("remote:{} code:{}", remote.name, code.name);
                                    } else {
                                        println!(
                                            "remote:{} unmapped code:{:x}",
                                            remote.name, decoded_code
                                        );
                                    }
                                } else {
                                    // lirc remote
                                    println!(
                                        "decoded: {}",
                                        var.iter()
                                            .map(|(name, val)| format!("{}={:x}", name, val))
                                            .join(", ")
                                    );
                                }
                            }
                        }
                    }
                }
            } else {
                log.error(&format!("{}: device cannot receive raw", lircdev));
                std::process::exit(1);
            }
        }
    }
}

fn process(raw: &[u32], irps: &[(Option<&Remote>, NFA)], matches: &clap::ArgMatches, log: &Log) {
    let graphviz = matches.is_present("GRAPHVIZ");

    for (remote, nfa) in irps {
        let mut matcher = nfa.matcher(100, 100);

        for (index, raw) in raw.iter().enumerate() {
            let ir = if index.is_odd() {
                InfraredData::Gap(*raw)
            } else {
                InfraredData::Flash(*raw)
            };

            if let Some(var) = matcher.input(ir) {
                if let Some(remote) = remote {
                    // lirc
                    let decoded_code = var["CODE"] as u64;

                    // TODO: raw codes
                    if let Some(code) = remote
                        .codes
                        .iter()
                        .find(|code| code.code[0] == decoded_code)
                    {
                        println!("remote:{} code:{}", remote.name, code.name);
                    } else {
                        println!("remote:{} unmapped code:{:x}", remote.name, decoded_code);
                    }
                } else {
                    // lirc remote
                    println!(
                        "decoded: {}",
                        var.iter()
                            .map(|(name, val)| format!("{}={:x}", name, val))
                            .join(", ")
                    );
                }
            }

            if graphviz {
                let filename = format!(
                    "{}_nfa_step_{:04}.dot",
                    if let Some(remote) = remote {
                        &remote.name
                    } else {
                        "irp"
                    },
                    index
                );

                log.info(&format!("saving nfa at step {} as {}", index, filename));

                matcher.dotgraphviz(&filename);
            }
        }
    }
}
