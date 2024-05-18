use proc_macros::SaveState;

#[derive(SaveState)]
struct Bar {
    z: u32,
}

impl PartialEq for BarState {
    fn eq(&self, other: &Self) -> bool {
        self.z == other.z
    }
}

impl Eq for BarState {}

#[allow(dead_code)]
#[derive(SaveState)]
struct Foo {
    a: u32,
    b: u64,
    #[save_state(skip)]
    c: i32,
    #[save_state(to = BarState)]
    d: Bar,
}

impl PartialEq for FooState {
    fn eq(&self, other: &Self) -> bool {
        self.a == other.a && self.b == other.b && self.d == other.d
    }
}

impl Eq for FooState {}

#[test]
fn foo_state() {
    let mut foo = Foo { a: 0, b: 1, c: -1, d: Bar { z: 2 } };

    assert_eq!(foo.save_state(), FooState { a: 0, b: 1, d: BarState { z: 2 } });
}
