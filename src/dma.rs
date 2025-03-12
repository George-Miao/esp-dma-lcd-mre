// Copied from https://github.com/Dominaezzz/esp-hal/commit/7ff621e68892c86821b45ec1a5408dd47f2e610c
// Reference: https://github.com/esp-rs/esp-hal/discussions/2866
//
use core::{cmp::min, ops::Range, ptr::null_mut};

use esp_hal::dma::{
    BurstConfig, DmaBufError, DmaDescriptor, DmaTxBuffer, Owner, Preparation, TransferDirection,
};

/// The lower bound of the system's DRAM (Data RAM) address space.
const SOC_DRAM_LOW: usize = 0x3FC8_8000;
/// The upper bound of the system's DRAM (Data RAM) address space.
const SOC_DRAM_HIGH: usize = 0x3FD0_0000;

const DRAM: Range<usize> = SOC_DRAM_LOW..SOC_DRAM_HIGH;

#[allow(unused)]
pub(crate) fn is_slice_in_dram<T>(slice: &[T]) -> bool {
    slice_in_range(slice, DRAM)
}

fn slice_in_range<T>(slice: &[T], range: Range<usize>) -> bool {
    let slice = slice.as_ptr_range();
    let start = slice.start as usize;
    let end = slice.end as usize;
    // `end` is >= `start`, so we don't need to check that `end > range.start`
    // `end` is also one past the last element, so it can be equal to the range's
    // end which is also one past the memory region's last valid address.
    range.contains(&start) && end <= range.end
}

/// DMA Streaming Transmit Buffer
pub struct DmaTxStreamBuf {
    descriptors: &'static mut [DmaDescriptor],
    buffer: &'static mut [u8],

    num_used_descriptors: usize,
    len_of_used_buffer: usize,
    buffer_write_offset: usize,
}

impl DmaTxStreamBuf {
    /// Creates a new [DmaTxStreamBuf].
    pub fn new(
        descriptors: &'static mut [DmaDescriptor],
        buffer: &'static mut [u8],
    ) -> Result<Self, DmaBufError> {
        if !is_slice_in_dram(descriptors) {
            return Err(DmaBufError::UnsupportedMemoryRegion);
        }
        if !is_slice_in_dram(buffer) {
            return Err(DmaBufError::UnsupportedMemoryRegion);
        }

        if descriptors.len() < 2 {
            return Err(DmaBufError::InsufficientDescriptors);
        }

        // The buffer starts out with one empty.
        descriptors.fill(DmaDescriptor::EMPTY);
        descriptors[0].set_owner(Owner::Dma);

        Ok(Self {
            descriptors,
            buffer,
            num_used_descriptors: 0,
            len_of_used_buffer: 0,
            buffer_write_offset: 0,
        })
    }

    /// Consume the buf, returning the descriptors and buffer.
    pub fn split(self) -> (&'static mut [DmaDescriptor], &'static mut [u8]) {
        (self.descriptors, self.buffer)
    }

    /// TODO
    pub fn push(&mut self, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }

        let max_chunk_size = BurstConfig::default().max_compatible_chunk_size();

        let num_free_descriptors = self.descriptors.len() - self.num_used_descriptors;

        let push_limit = min(
            max_chunk_size * num_free_descriptors,
            self.buffer.len() - self.len_of_used_buffer,
        );
        let available_space = push_limit.saturating_sub(self.buffer_write_offset);

        let available_buffer = &mut self.buffer[self.len_of_used_buffer..]
            [self.buffer_write_offset..][..available_space];

        let bytes_to_push = min(data.len(), available_buffer.len());

        available_buffer[..bytes_to_push].copy_from_slice(&data[..bytes_to_push]);
        self.buffer_write_offset += bytes_to_push;

        bytes_to_push
    }

    fn commit(&mut self, with_eof: bool) {
        if self.buffer_write_offset == 0 {
            return;
        }

        let max_chunk_size = BurstConfig::default().max_compatible_chunk_size();

        let mut free_descriptors = self.descriptors.iter_mut().skip(self.num_used_descriptors);
        let mut remaining_to_commit =
            &mut self.buffer[self.len_of_used_buffer..][..self.buffer_write_offset];

        let mut used_descriptors = 0;
        while !remaining_to_commit.is_empty() {
            let chunk_size = max_chunk_size.min(remaining_to_commit.len());
            let (chunk, remaining) = remaining_to_commit.split_at_mut(chunk_size);

            let desc = free_descriptors.next().unwrap();

            desc.set_owner(Owner::Dma);
            desc.set_length(chunk.len());
            desc.set_size(chunk.len());
            desc.set_suc_eof(false);
            desc.buffer = chunk.as_mut_ptr();

            remaining_to_commit = remaining;
            used_descriptors += 1;
        }

        let num_unused_descriptors = free_descriptors.count();

        let descriptors_to_link = self
            .descriptors
            .iter_mut()
            .rev()
            .skip(num_unused_descriptors)
            .take(used_descriptors + 1);

        let mut next = null_mut();
        for desc in descriptors_to_link {
            desc.next = next;
            next = desc;
        }

        self.num_used_descriptors += used_descriptors;
        self.len_of_used_buffer += self.buffer_write_offset;
        self.buffer_write_offset = 0;

        self.descriptors[self.num_used_descriptors.saturating_sub(1)].set_suc_eof(with_eof);
    }
}

unsafe impl DmaTxBuffer for DmaTxStreamBuf {
    type View = DmaTxStreamBufView;

    fn prepare(&mut self) -> Preparation {
        self.commit(false);

        Preparation {
            start: self.descriptors.as_mut_ptr(),
            direction: TransferDirection::Out,
            accesses_psram: false,
            check_owner: None,
            burst_transfer: BurstConfig::default(),
            auto_write_back: true,
        }
    }

    fn into_view(self) -> Self::View {
        DmaTxStreamBufView {
            buffer_idx: self.len_of_used_buffer % self.buffer.len(),
            free_descriptors: self.descriptors.len() - self.num_used_descriptors,
            free_buffer_space: self.buffer.len() - self.len_of_used_buffer,
            descriptors: self.descriptors,
            buffer: self.buffer,
            descriptor_idx: self.num_used_descriptors,
        }
    }

    fn from_view(view: Self::View) -> Self {
        let descriptors = view.descriptors;
        let buffer = view.buffer;

        // The buffer starts out with one empty.
        descriptors.fill(DmaDescriptor::EMPTY);
        descriptors[0].set_owner(Owner::Dma);

        Self {
            descriptors,
            buffer,
            num_used_descriptors: 0,
            len_of_used_buffer: 0,
            buffer_write_offset: 0,
        }
    }
}

/// A view into a [DmaTxStreamBuf].
pub struct DmaTxStreamBufView {
    descriptors: &'static mut [DmaDescriptor],
    buffer: &'static mut [u8],

    // Index of descriptor to use for next push.
    descriptor_idx: usize,
    // Position in buffer to start writing to for next push.
    buffer_idx: usize,

    free_descriptors: usize,
    free_buffer_space: usize,
}

impl DmaTxStreamBufView {
    /// TODO
    pub fn push(&mut self, data: &[u8], set_eof: bool) -> usize {
        if data.is_empty() {
            // TODO: Handle EOF
            return 0;
        }

        let max_chunk_size = BurstConfig::default().max_compatible_chunk_size();

        let mut remaining_to_push = data;
        while !remaining_to_push.is_empty() {
            let chunk_size = min(max_chunk_size, self.buffer.len() - self.buffer_idx);
            let max_to_push = chunk_size.min(remaining_to_push.len());

            // info!("attempting reclaim? {} {} {} {} {} {} {} {}", self.free_descriptors,
            // self.free_buffer_space, max_to_push, chunk_size, max_chunk_size,
            // self.buffer.len(), self.buffer_idx, remaining_to_push.len());
            // panic!();
            if self.free_descriptors == 0 || self.free_buffer_space < max_to_push {
                self.reclaim_from_dma();
            }
            if self.free_descriptors == 0 || self.free_buffer_space == 0 {
                break;
            }

            let chunk_size = min(max_to_push, self.free_buffer_space);

            let (chunk, remaining) = remaining_to_push.split_at(chunk_size);

            let buffer_len = self.buffer.len();
            let dest = &mut self.buffer[self.buffer_idx..][..chunk_size];
            dest.copy_from_slice(chunk);
            self.free_buffer_space -= chunk_size;
            self.buffer_idx += chunk_size;
            if self.buffer_idx == buffer_len {
                self.buffer_idx = 0;
            }

            let descriptor = &mut self.descriptors[self.descriptor_idx];

            descriptor.next = null_mut();
            descriptor.buffer = dest.as_mut_ptr();
            descriptor.set_length(chunk.len());
            descriptor.set_size(chunk.len());
            descriptor.set_suc_eof(set_eof && remaining.is_empty());
            descriptor.set_owner(Owner::Dma);

            let descriptor: *mut _ = descriptor;
            self.descriptors[self
                .descriptor_idx
                .checked_sub(1)
                .unwrap_or(self.descriptors.len() - 1)]
            .next = descriptor;

            self.free_descriptors -= 1;
            self.descriptor_idx += 1;
            if self.descriptor_idx == self.descriptors.len() {
                self.descriptor_idx = 0;
            }
            remaining_to_push = remaining;
        }

        data.len() - remaining_to_push.len()
    }

    /// TODO
    pub fn available_bytes(&self) -> usize {
        todo!()
    }

    fn reclaim_from_dma(&mut self) {
        let (last, first) = self.descriptors.split_at(self.descriptor_idx);
        let descriptors_to_reclaim = first.iter().chain(last.iter()).skip(self.free_descriptors);

        let buffer_start = self.buffer.as_ptr();
        let buffer_end = unsafe { self.buffer.as_ptr().add(self.buffer.len()) };
        let mut buffer_checkpoint = unsafe {
            self.buffer
                .as_mut_ptr()
                .add((self.buffer_idx + self.free_buffer_space) / self.buffer.len())
        };

        for descriptor in descriptors_to_reclaim {
            if descriptor.owner() == Owner::Dma {
                break;
            }

            if descriptor.buffer >= buffer_checkpoint {
                let new_checkpoint = unsafe { descriptor.buffer.add(descriptor.size()) };
                self.free_buffer_space +=
                    unsafe { new_checkpoint.offset_from(buffer_checkpoint) } as usize;
                buffer_checkpoint = new_checkpoint;
            } else {
                self.free_buffer_space +=
                    unsafe { buffer_end.offset_from(buffer_checkpoint) } as usize;
                let new_checkpoint = unsafe { descriptor.buffer.add(descriptor.size()) };
                self.free_buffer_space +=
                    unsafe { new_checkpoint.offset_from(buffer_start) } as usize;
                buffer_checkpoint = new_checkpoint;
            }
            self.free_descriptors += 1;
        }
    }
}
