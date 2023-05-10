use super::{
    build_nfa::{Action, Edge, Vertex},
    Vartable,
};
use itertools::Itertools;
use std::{char, fs::File, io::Write, path::PathBuf};

/// Generate a GraphViz dot file and write to the given path
pub(crate) fn graphviz(verts: &[Vertex], name: &str, states: &[(usize, Vartable)], path: &str) {
    let path = PathBuf::from(path);
    let mut file = File::create(path).expect("create file");

    writeln!(&mut file, "strict digraph {name} {{").unwrap();

    let mut vert_names = Vec::new();

    for (no, v) in verts.iter().enumerate() {
        let name = if v.actions.iter().any(|a| matches!(a, Action::Done(..))) {
            format!("done ({no})")
        } else {
            format!("{} ({})", no_to_name(vert_names.len()), no)
        };

        let mut labels: Vec<String> = v
            .actions
            .iter()
            .map(|a| match a {
                Action::Set { var, expr } => format!("{var} = {expr}"),
                Action::AssertEq { left, right } => format!("assert {left} = {right}",),
                Action::Done(event, res) => format!("{} ({})", event, res.iter().join(", ")),
            })
            .collect::<Vec<String>>();

        if let Some(Edge::BranchCond { expr, .. }) = v
            .edges
            .iter()
            .find(|e| matches!(e, Edge::BranchCond { .. }))
        {
            labels.push(format!("cond: {expr}"));
        }

        if let Some(Edge::MayBranchCond { expr, .. }) = v
            .edges
            .iter()
            .find(|e| matches!(e, Edge::MayBranchCond { .. }))
        {
            labels.push(format!("may cond: {expr}"));
        }

        let color = if let Some((_, vars)) = states.iter().find(|(node, _)| *node == no) {
            let values = vars
                .vars
                .iter()
                .map(|(name, (val, _))| format!("{name}={val}"))
                .collect::<Vec<String>>();

            labels.push(format!("state: {}", values.join(", ")));

            " [color=red]"
        } else {
            ""
        };

        if !labels.is_empty() {
            writeln!(
                &mut file,
                "\t\"{}\" [label=\"{}\\n{}\"]{}",
                name,
                name,
                labels.join("\\n"),
                color
            )
            .unwrap();
        } else if !color.is_empty() {
            writeln!(&mut file, "\t\"{name}\"{color}").unwrap();
        }

        vert_names.push(name);
    }

    for (i, v) in verts.iter().enumerate() {
        for edge in &v.edges {
            match edge {
                Edge::Flash {
                    length,
                    complete,
                    dest,
                } => writeln!(
                    &mut file,
                    "\t\"{}\" -> \"{}\" [label=\"flash {} {}\"]",
                    vert_names[i],
                    vert_names[*dest],
                    length,
                    if *complete { " complete" } else { "" }
                )
                .unwrap(),
                Edge::Gap {
                    length,
                    complete,
                    dest,
                } => writeln!(
                    &mut file,
                    "\t\"{}\" -> \"{}\" [label=\"gap {} {}\"]",
                    vert_names[i],
                    vert_names[*dest],
                    length,
                    if *complete { " complete" } else { "" }
                )
                .unwrap(),
                Edge::BranchCond { yes, no, .. } => {
                    writeln!(
                        &mut file,
                        "\t\"{}\" -> \"{}\" [label=\"cond: true\"]",
                        vert_names[i], vert_names[*yes]
                    )
                    .unwrap();
                    //

                    writeln!(
                        &mut file,
                        "\t\"{}\" -> \"{}\" [label=\"cond: false\"]",
                        vert_names[i], vert_names[*no]
                    )
                    .unwrap();
                }
                Edge::MayBranchCond { dest, .. } => {
                    writeln!(
                        &mut file,
                        "\t\"{}\" -> \"{}\" [label=\"may branch\"]",
                        vert_names[i], vert_names[*dest]
                    )
                    .unwrap();
                }
                Edge::Branch(dest) => writeln!(
                    &mut file,
                    "\t\"{}\" -> \"{}\"",
                    vert_names[i], vert_names[*dest]
                )
                .unwrap(),
            }
        }
    }

    writeln!(&mut file, "}}").unwrap();
}

fn no_to_name(no: usize) -> String {
    let mut no = no;
    let mut res = String::new();

    loop {
        let ch = char::from_u32((65 + no % 26) as u32).unwrap();

        res.insert(0, ch);

        no /= 26;
        if no == 0 {
            return res;
        }
    }
}
