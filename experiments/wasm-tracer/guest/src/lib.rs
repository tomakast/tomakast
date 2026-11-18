//! Guest side of the WASM tracer bullet: one `search` export plus two misbehaving exports.

wit_bindgen::generate!({
    world: "tracer",
    path: "../wit",
});

struct Tracer;

impl Guest for Tracer {
    fn search(query: String) -> Vec<Item> {
        let n: usize = query
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(0);
        (0..n)
            .map(|i| Item {
                id: format!("tracer/{i}"),
                title: format!("Result {i} for {query}"),
                subtitle: "A small record, about the size of a launcher row".to_string(),
                score: i as u32,
            })
            .collect()
    }

    fn spin() {
        let mut x: u64 = 0;
        loop {
            x = std::hint::black_box(x.wrapping_add(1));
        }
    }

    fn hog() {
        const SIZE: usize = 1 << 30;
        // Fill with a non-zero byte so every page is written and nothing can be elided.
        let mut buf: Vec<u8> = Vec::with_capacity(SIZE);
        buf.resize(SIZE, 1);
        std::hint::black_box(&buf);
    }
}

export!(Tracer);
