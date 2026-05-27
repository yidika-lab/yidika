use std::sync::mpsc;
use std::thread;
use std::time::Instant;

fn main() {
    let (tx1, rx1) = mpsc::channel::<()>();
    let (tx2, rx2) = mpsc::channel::<()>();
    
    let jh = thread::spawn(move || {
        loop {
            match rx1.recv() {
                Ok(()) => {
                    let _ = tx2.send(());
                }
                Err(_) => break,
            }
        }
    });

    // Warm up
    for _ in 0..10 {
        let _ = tx1.send(());
        let _ = rx2.recv();
    }

    let mut times = Vec::new();
    for _ in 0..100 {
        let t0 = Instant::now();
        let _ = tx1.send(());
        let _ = rx2.recv();
        times.push(t0.elapsed());
    }
    
    drop(tx1);
    jh.join().unwrap();

    let avg = times.iter().map(|d| d.as_nanos()).sum::<u128>() / times.len() as u128;
    let min = times.iter().map(|d| d.as_nanos()).min().unwrap();
    let max = times.iter().map(|d| d.as_nanos()).max().unwrap();
    println!("avg={}ns min={}ns max={}ns", avg, min, max);
}
