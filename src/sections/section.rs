use std::cmp::Ordering;
use crate::NumberStyle;

pub trait Section {
    fn print_info(&self, style: NumberStyle);

    fn start(&self) -> u64;

    fn size(&self) -> usize;

    fn end(&self) -> u64 {
        self.start() + self.size() as u64 - 1
    }

    fn compare_offset(&self, offset: u64) -> Ordering {
        if self.end() < offset {
            Ordering::Less
        } else if self.start() > offset {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}
