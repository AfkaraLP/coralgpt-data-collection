use std::{collections::VecDeque, ops::Deref};

#[derive(Clone, Default, Debug)]
pub struct FifoBuffer<const N: usize, T> {
    buf: VecDeque<T>,
}

impl<const N: usize, T> FifoBuffer<N, T> {
    pub fn new() -> Self {
        Self {
            buf: VecDeque::with_capacity(N),
        }
    }

    pub fn push(&mut self, item: T) {
        if self.buf.len() == N {
            self.buf.pop_back();
        }
        self.buf.push_front(item);
    }

    pub fn pop(&mut self) -> Option<T> {
        self.buf.pop_back()
    }
}

impl<const N: usize, T> Deref for FifoBuffer<N, T> {
    type Target = VecDeque<T>;

    fn deref(&self) -> &Self::Target {
        &self.buf
    }
}
