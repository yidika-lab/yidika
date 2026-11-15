pub fn parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let work = &work;
    std::thread::scope(|scope| {
        let handles: Vec<_> = items.iter().map(|item| scope.spawn(move || work(item))).collect();
        handles.into_iter().map(|h| h.join().expect("package worker panicked")).collect()
    })
}
