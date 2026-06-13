#[derive(Debug, Clone)]
struct Counter {
    value: i32,
}

impl Counter {
    fn add(&mut self, delta: i32) {
        self.value += delta;
    }

    fn reset(&mut self) {
        self.value = 0;
    }
}

fn main() {
    let mut counter = Counter { value: 1 };
    counter.add(2);
    println!("{counter:?}");
}
