//! A ledger for memory mappings.

#![cfg_attr(not(test), no_std)]
#![deny(clippy::all)]
#![deny(missing_docs)]
#![forbid(unsafe_code)]

use core::cmp::Ordering;

use lset::{Empty, Line, Span};
use primordial::{Address, Offset, Page};

/// A ledger region.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Region<V> {
    /// Limits for the region.
    pub limits: Line<Address<usize, Page>>,
    /// Fill value for the region.
    pub value: Option<V>,
}

impl<V: Sized> Region<V> {
    #[inline]
    const fn new(limits: Line<Address<usize, Page>>, value: Option<V>) -> Self {
        Self { limits, value }
    }

    #[inline]
    const fn empty() -> Self {
        Self::new(Line::new(Address::NULL, Address::NULL), None)
    }
}

/// Ledger error conditions.
#[derive(Debug)]
pub enum Error {
    /// Out of storage capacity
    OutOfCapacity,

    /// No space for the region
    OutOfSpace,

    /// Overlapping with the existing regions
    Overlap,

    /// Invalid region given as input
    InvalidRegion,
}

/// A virtual memory map ledger.
#[derive(Clone, Debug)]
pub struct Ledger<V, const N: usize> {
    limits: Line<Address<usize, Page>>,
    regions: [Region<V>; N],
    len: usize,
}

impl<V: Eq + Copy, const N: usize> Ledger<V, N> {
    /// Sort the regions.
    fn sort(&mut self) {
        self.regions_mut().sort_unstable_by(|l, r| {
            if l.limits == r.limits {
                Ordering::Equal
            } else if l.limits.is_empty() {
                Ordering::Greater
            } else if r.limits.is_empty() {
                Ordering::Less
            } else {
                l.limits.start.cmp(&r.limits.start)
            }
        })
    }

    /// Create a new instance.
    pub fn new(limits: Line<Address<usize, Page>>) -> Self {
        Self {
            limits,
            regions: [Region::empty(); N],
            len: 0,
        }
    }

    /// Get an immutable view of the regions.
    pub fn regions(&self) -> &[Region<V>] {
        &self.regions[..self.len]
    }

    /// Get a mutable view of the regions.
    fn regions_mut(&mut self) -> &mut [Region<V>] {
        &mut self.regions[..self.len]
    }

    /// Insert a new region into the ledger.
    pub fn insert(&mut self, region: Region<V>) -> Result<(), Error> {
        if region.limits.start >= region.limits.end {
            return Err(Error::InvalidRegion);
        }

        // Make sure the region fits in our adress space.
        if region.limits.start < self.limits.start || region.limits.end > self.limits.end {
            return Err(Error::InvalidRegion);
        }

        // Loop over the regions looking for merges.
        let mut iter = self.regions_mut().iter_mut().peekable();
        while let Some(prev) = iter.next() {
            if prev.limits.intersection(region.limits).is_some() {
                return Err(Error::Overlap);
            }

            if let Some(next) = iter.peek() {
                if next.limits.intersection(region.limits).is_some() {
                    return Err(Error::Overlap);
                }
            }

            // Merge previous.
            if prev.value == region.value && prev.limits.end == region.limits.start {
                prev.limits.end = region.limits.end;
                return Ok(());
            }

            // Merge next.
            if let Some(next) = iter.peek_mut() {
                if next.value == region.value && next.limits.start == region.limits.end {
                    next.limits.start = region.limits.start;
                    return Ok(());
                }
            }
        }

        if self.len < self.regions.len() {
            self.regions[self.len] = region;
            self.len += 1;
            self.sort();
            return Ok(());
        }

        Err(Error::OutOfCapacity)
    }

    /// Find space for a region.
    pub fn find_free(
        &self,
        len: Offset<usize, Page>,
        front: bool,
    ) -> Result<Line<Address<usize, Page>>, Error> {
        let start = Region::<V>::new(Line::new(self.limits.start, self.limits.start), None);
        let end = Region::<V>::new(Line::new(self.limits.end, self.limits.end), None);
        let first = [start, *self.regions().first().unwrap_or(&end)];
        let last = [*self.regions().last().unwrap_or(&start), end];

        // Chain everything together.
        let mut iter = first
            .windows(2)
            .chain(self.regions().windows(2))
            .chain(last.windows(2));

        // Iterate through the windows.
        if front {
            while let Some([l, r]) = iter.next() {
                if r.limits.end - l.limits.start > len {
                    return Ok(Span::new(l.limits.end, len).into());
                }
            }
        } else {
            let mut iter = iter.rev();
            while let Some([l, r]) = iter.next() {
                if r.limits.end - l.limits.start > len {
                    return Ok(Span::new(r.limits.start - len, len).into());
                }
            }
        }

        Err(Error::OutOfSpace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Line<Address<usize, Page>> =
        Line::new(Address::new(0x1000), Address::new(0x10000));

    #[test]
    fn insert() {
        const X: Line<Address<usize, Page>> =
            Line::new(Address::new(0xe000), Address::new(0x10000));

        let mut ledger: Ledger<(), 1> = Ledger::new(LIMITS);
        assert_eq!(ledger.len, 0);
        ledger.insert(Region::new(X, None)).unwrap();
        assert_eq!(ledger.regions(), &[Region::<()>::new(X, None)]);
    }

    #[test]
    fn find_free_front() {
        const D: Offset<usize, Page> = Offset::from_items(2);
        const A: Line<Address<usize, Page>> = Line::new(Address::new(0x1000), Address::new(0x3000));
        const B: Line<Address<usize, Page>> = Line::new(Address::new(0x3000), Address::new(0x5000));

        let mut ledger: Ledger<(), 8> = Ledger::new(LIMITS);
        assert_eq!(ledger.find_free(D, true).unwrap(), A);
        ledger.insert(Region::new(A, None)).unwrap();
        assert_eq!(ledger.find_free(D, true).unwrap(), B);
    }

    #[test]
    fn find_free_back() {
        const D: Offset<usize, Page> = Offset::from_items(2);
        const A: Line<Address<usize, Page>> =
            Line::new(Address::new(0xe000), Address::new(0x10000));
        const B: Line<Address<usize, Page>> = Line::new(Address::new(0xc000), Address::new(0xe000));

        let mut ledger: Ledger<(), 8> = Ledger::new(LIMITS);
        assert_eq!(ledger.find_free(D, false).unwrap(), A);
        ledger.insert(Region::new(A, None)).unwrap();
        assert_eq!(ledger.find_free(D, false).unwrap(), B);
    }

    #[test]
    fn merge_after() {
        const A: Line<Address<usize, Page>> = Line::new(Address::new(0x4000), Address::new(0x5000));
        const B: Line<Address<usize, Page>> = Line::new(Address::new(0x8000), Address::new(0x9000));

        const X: Line<Address<usize, Page>> = Line::new(Address::new(0x5000), Address::new(0x6000));
        const Y: Line<Address<usize, Page>> = Line::new(Address::new(0x4000), Address::new(0x6000));

        let mut ledger: Ledger<(), 8> = Ledger::new(LIMITS);
        ledger.insert(Region::new(A, None)).unwrap();
        ledger.insert(Region::new(B, None)).unwrap();
        ledger.insert(Region::new(X, None)).unwrap();

        assert_eq!(ledger.len, 2);
        assert_eq!(ledger.regions[0], Region::new(Y, None));
        assert_eq!(ledger.regions[1].limits, B);
    }

    #[test]
    fn merge_before() {
        const A: Line<Address<usize, Page>> = Line::new(Address::new(0x4000), Address::new(0x5000));
        const B: Line<Address<usize, Page>> = Line::new(Address::new(0x8000), Address::new(0x9000));

        const X: Line<Address<usize, Page>> = Line::new(Address::new(0x7000), Address::new(0x8000));
        const Y: Line<Address<usize, Page>> = Line::new(Address::new(0x7000), Address::new(0x9000));

        let mut ledger: Ledger<(), 8> = Ledger::new(LIMITS);
        ledger.insert(Region::new(A, None)).unwrap();
        ledger.insert(Region::new(B, None)).unwrap();
        ledger.insert(Region::new(X, None)).unwrap();

        assert_eq!(ledger.len, 2);
        assert_eq!(ledger.regions[0].limits, A);
        assert_eq!(ledger.regions[1], Region::new(Y, None));
    }
}
