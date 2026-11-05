//! Export the actual Move graph offline; uses exactly the package's parser/planner/builder.
use bgx_comfy_wan22_example::{graph, parameters, planner};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("Pass request-move.json")?;
    let input = serde_json::from_slice(&std::fs::read(path)?)?;
    let request = parameters::parse(&input, parameters::Mode::Move)
        .map_err(|e| format!("{}: {}", e.code, e.message))?;
    let plan = planner::plan(&request).map_err(|e| format!("{}: {}", e.code, e.message))?;
    let graph = graph::build(&request, &plan, &graph::Inputs {
        source_file: "source.mp4", replacement_file: "character.jpg", points: &[],
    });
    println!("{}", serde_json::to_string_pretty(&graph)?);
    Ok(())
}
