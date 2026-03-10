use std::{collections::VecDeque, ops::Deref};

use azalea::FormattedText;

use crate::IsSpamMessage__PiperizeTrait;

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

pub trait ChatFifoOps {
    fn get_past_n_messages(&self, verbose: bool, n: usize) -> Vec<String>;
    // fn iter_past_n_messages(&self, verbose: bool, n: usize) -> impl Iterator<Item = String>;
}

impl<const N: usize> ChatFifoOps for FifoBuffer<N, FormattedText> {
    fn get_past_n_messages(&self, verbose: bool, n: usize) -> Vec<String> {
        self.iter()
            .map(ToString::to_string)
            .filter(|v| !v.is_spam_message() || verbose)
            .take(n)
            .collect::<Vec<String>>()
            .iter()
            .rev()
            .map(ToOwned::to_owned)
            .collect()
    }
}
