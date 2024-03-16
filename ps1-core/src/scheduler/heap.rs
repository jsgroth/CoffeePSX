use std::array;

#[derive(Debug, Clone)]
pub struct UpdateableMinHeap<T, const CAPACITY: usize> {
    heap: [T; CAPACITY],
    len: usize,
}

impl<T: Copy + Default + Ord, const CAPACITY: usize> UpdateableMinHeap<T, CAPACITY> {
    pub fn new() -> Self {
        Self { heap: array::from_fn(|_| T::default()), len: 0 }
    }

    pub fn push(&mut self, value: T) {
        assert!(self.len < CAPACITY, "Push while heap is at capacity of {CAPACITY}");

        self.heap[self.len] = value;
        self.len += 1;

        self.reheapify_down(self.len - 1);
    }

    pub fn peek(&self) -> T {
        self.heap[0]
    }

    pub fn pop(&mut self) -> T {
        assert_ne!(self.len, 0, "Pop while heap is empty");

        self.len -= 1;
        self.heap.swap(0, self.len);

        self.reheapify_up(0);

        let value = self.heap[self.len];
        if self.len == 0 {
            self.heap[0] = T::default();
        }

        value
    }

    pub fn update_or_push(&mut self, new_value: T, pred: impl Fn(T) -> bool) {
        let Some(i) = self.heap[..self.len].iter().position(|&value| pred(value)) else {
            self.push(new_value);
            return;
        };

        let old_value = self.heap[i];
        self.heap[i] = new_value;

        if new_value < old_value {
            self.reheapify_down(i);
        } else {
            self.reheapify_up(i);
        }
    }

    pub fn remove_one(&mut self, pred: impl Fn(T) -> bool) {
        let Some(i) = self.heap[..self.len].iter().position(|&value| pred(value)) else {
            return;
        };

        let old_value = self.heap[i];
        self.len -= 1;
        self.heap.swap(i, self.len);

        let new_value = self.heap[i];
        if new_value < old_value {
            self.reheapify_down(i);
        } else {
            self.reheapify_up(i);
        }

        if self.len == 0 {
            self.heap[0] = T::default();
        }
    }

    fn reheapify_down(&mut self, mut i: usize) {
        while i != 0 {
            let j = (i - 1) / 2;
            if self.heap[j] <= self.heap[i] {
                break;
            }

            self.heap.swap(i, j);
            i = j;
        }
    }

    fn reheapify_up(&mut self, mut i: usize) {
        loop {
            let j1 = 2 * i + 1;
            let j2 = 2 * i + 2;

            if j1 >= self.len {
                break;
            }

            if self.heap[i] <= self.heap[j1] && (j2 >= self.len || self.heap[i] <= self.heap[j2]) {
                break;
            }

            if j2 < self.len && self.heap[j2] < self.heap[j1] {
                self.heap.swap(i, j2);
                i = j2;
            } else {
                self.heap.swap(i, j1);
                i = j1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAPACITY: usize = 20;

    fn new_heap() -> UpdateableMinHeap<i64, CAPACITY> {
        UpdateableMinHeap::new()
    }

    fn random_numbers() -> [i64; CAPACITY] {
        array::from_fn(|_| rand::random())
    }

    #[test]
    fn basic_functionality() {
        let mut heap = new_heap();

        for _ in 0..100 {
            let mut numbers = random_numbers();
            for number in numbers {
                heap.push(number);
            }

            numbers.sort();
            for number in numbers {
                assert_eq!(heap.pop(), number);
            }
        }
    }

    #[test]
    fn update() {
        let mut heap = new_heap();

        for _ in 0..100 {
            let mut numbers = random_numbers();
            for number in numbers {
                heap.push(number);
            }

            let prev_value = numbers[0];
            numbers[0] = rand::random();
            heap.update_or_push(numbers[0], |value| value == prev_value);

            numbers.sort();
            for number in numbers {
                assert_eq!(heap.pop(), number);
            }
        }
    }

    #[test]
    fn update_no_match() {
        let mut heap = new_heap();

        for _ in 0..100 {
            let mut numbers = random_numbers();

            for &number in &numbers[1..] {
                heap.push(number);
            }

            heap.update_or_push(numbers[0], |_| false);

            numbers.sort();
            for number in numbers {
                assert_eq!(heap.pop(), number);
            }
        }
    }

    #[test]
    fn remove() {
        let mut heap = new_heap();

        for _ in 0..100 {
            let mut numbers = random_numbers();
            for number in numbers {
                heap.push(number);
            }

            heap.remove_one(|value| value == numbers[0]);

            numbers[1..].sort();
            for &number in &numbers[1..] {
                assert_eq!(heap.pop(), number);
            }
        }
    }
}
