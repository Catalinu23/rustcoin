use std::env;
use std::fs::File;
use std::process::exit;

use lib::types::Block;
use lib::util::Saveable;

fn main() {
    let (path, steps) = if let (Some(arg1), Some(arg2)) = (env::args().nth(1), env::args().nth(2)) {
        (arg1, arg2)
    } else {
        eprintln!("Usage: miner <path> <steps>");
        exit(1);
    };
    let steps: usize = if let Ok(s @ 1..=usize::MAX) = steps.parse() {
        s
    } else {
        eprintln!("Invalid steps: {}", steps);
        exit(1);
    };

    let file = File::open(&path).expect("Failed to open block file");
    let og_block = Block::load(file).expect("Failed to load block from file");
    let mut block = og_block.clone();

    while !block.header.mine(steps) {
        println!("Mining...");
        println!("original: {:#?}", og_block);
        println!("hash: {}", og_block.header.hash());
        println!("final: {:#?}", block);
        println!("hash: {}", block.header.hash());
    }

    println!("Mined!");
    println!("hash: {}", block.header.hash());
    println!("nonce: {}", block.header.nonce);
}
