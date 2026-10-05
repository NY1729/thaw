#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn mapped_callback_results_preserve_hole_and_undefined_states() {
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(Some(allocate)));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let result = mapped_array_result_with_states(&[Some(11), Some(0), None], &[1, 2, 0]);
        let handle = (result.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
        assert_ne!(result.to_bits() & SPARSE_ARRAY_RESULT_TAG, 0);
        assert_eq!(array_index_state(result, 0), 1);
        assert_eq!(array_index_state(result, 1), 2);
        assert_eq!(array_index_state(result, 2), 0);
        unsafe {
            libc::free(handle.read().cast());
            libc::free(handle.add(1).read().cast());
            libc::free(handle.cast());
        }
        assert!(CALL_ERROR.with(|error| error.get().is_null()));
        CALL_ERROR.with(|error| error.set(previous_error));
        ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn basic_number_callbacks_distinguish_holes_from_values() {
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        let data = [3_u64, 10.0_f64.to_bits(), (-5.0_f64).to_bits(), 30.0_f64.to_bits()];
        let mask = [3_u64, 0x010001];
        let handle = [data.as_ptr().cast::<u8>(), mask.as_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_ptr() as usize as u64);
        let previous_query = ARRAY_INDEX_PRESENT.with(|query| query.replace(Some(present)));
        let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(Some(allocate)));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let previous_present = CALL_PRESENT.with(|present| present.replace(true));
        assert_eq!(number_array_every_gt(value, 0.0), 1.0);
        assert_eq!(number_array_find_index_ne(value, 10.0), 1.0);
        assert_eq!(number_array_find_ne(value, 10.0), 0.0);
        assert!(!CALL_PRESENT.with(Cell::get));
        CALL_PRESENT.with(|present| present.set(true));
        let filtered = number_array_filter_gt(value, 0.0);
        let output = (filtered.to_bits() & !ARRAY_RESULT_TAG) as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 2);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 30.0);
        assert!(CALL_ERROR.with(|error| error.get().is_null()));
        unsafe { libc::free(output.cast()) };
        CALL_PRESENT.with(|present| present.set(previous_present));
        CALL_ERROR.with(|error| error.set(previous_error));
        ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
        ARRAY_INDEX_PRESENT.with(|query| query.set(previous_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn mapped_sparse_result_keeps_distinct_value_and_presence_buffers() {
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn search(
            _: u8, _: *const u8, presence: *const u8, _: f64, _: f64,
        ) -> f64 {
            if presence.is_null() { return -9.0; }
            let length = unsafe { presence.cast::<u64>().read() };
            let states = unsafe {
                [presence.add(8).read(), presence.add(9).read(), presence.add(10).read()]
            };
            if length == 3 && states == [1, 0, 1] { 2.0 } else { -8.0 }
        }
        unsafe extern "C" fn format(
            _: u8, _: *const u8, presence: *const u8, _: *const c_char,
        ) -> *const c_char {
            if presence.is_null() { return c"missing-mask".as_ptr(); }
            let states = unsafe {
                [presence.add(8).read(), presence.add(9).read(), presence.add(10).read()]
            };
            if states == [1, 0, 1] { c"masked".as_ptr() } else { c"wrong-mask".as_ptr() }
        }
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn slice_data(
            array: *const u8, width: usize, start: f64, end: f64,
        ) -> *mut u8 {
            assert_eq!((width, start, end), (8, 0.0, 2.0));
            let output = unsafe { libc::malloc(24).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(2);
                std::ptr::copy_nonoverlapping(array.add(8), output.add(8), 16);
            }
            output
        }
        unsafe extern "C" fn slice_presence(
            presence: *const u8, length: usize, start: f64, end: f64,
        ) -> *mut u8 {
            assert_eq!((length, start, end), (3, 0.0, 2.0));
            assert_eq!(unsafe { presence.add(9).read() }, 0);
            let output = unsafe { libc::malloc(10).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write_unaligned(2);
                output.add(8).write(1);
                output.add(9).write(0);
            }
            output
        }
        unsafe extern "C" fn concat_data(
            left: *const u8, right: *const u8, width: usize,
        ) -> *mut u8 {
            assert_eq!(width, 8);
            let output = unsafe { libc::malloc(56).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(6);
                std::ptr::copy_nonoverlapping(left.add(8), output.add(8), 24);
                std::ptr::copy_nonoverlapping(right.add(8), output.add(32), 24);
            }
            output
        }
        unsafe extern "C" fn append_data(
            operation: u8, array: *const u8, value: f64,
        ) -> *mut u8 {
            assert_eq!((operation, value), (0, 40.0));
            let length = unsafe { array.cast::<u64>().read() as usize };
            let output = unsafe { libc::malloc(8 + (length + 1) * 8).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write((length + 1) as u64);
                std::ptr::copy_nonoverlapping(array.add(8), output.add(8), length * 8);
                output.add(8 + length * 8).cast::<f64>().write_unaligned(value);
            }
            output
        }
        unsafe extern "C" fn reverse_data(array: *const u8, width: usize) -> *mut u8 {
            assert_eq!(width, 8);
            let length = unsafe { array.cast::<u64>().read() as usize };
            let output = unsafe { libc::malloc(8 + length * 8).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(length as u64);
                for index in 0..length {
                    std::ptr::copy_nonoverlapping(
                        array.add(8 + (length - 1 - index) * 8),
                        output.add(8 + index * 8),
                        8,
                    );
                }
            }
            output
        }
        unsafe extern "C" fn reverse_in_place(array: *mut u8, width: usize) -> *mut u8 {
            assert_eq!(width, 8);
            let length = unsafe { array.cast::<u64>().read() as usize };
            for left in 0..length / 2 {
                unsafe {
                    std::ptr::swap_nonoverlapping(
                        array.add(8 + left * width),
                        array.add(8 + (length - left - 1) * width), width,
                    );
                }
            }
            array
        }
        unsafe extern "C" fn with_data(
            operation: u8, array: *const u8, index: f64, value: f64,
        ) -> *mut u8 {
            assert_eq!((operation, index, value), (0, 1.0, 40.0));
            let length = unsafe { array.cast::<u64>().read() as usize };
            let output = unsafe { libc::malloc(8 + length * 8).cast::<u8>() };
            unsafe {
                std::ptr::copy_nonoverlapping(array, output, 8 + length * 8);
                output.add(8 + 8).cast::<f64>().write_unaligned(value);
            }
            output
        }
        let data = [3_u64, 10.0_f64.to_bits(), 99.0_f64.to_bits(), 30.0_f64.to_bits()];
        let mask = [3_u64, 0x010001];
        let handle = [data.as_ptr().cast::<u8>(), mask.as_ptr().cast::<u8>()];
        let source = f64::from_bits(handle.as_ptr() as usize as u64);
        let previous_query = ARRAY_INDEX_PRESENT.with(|query| query.replace(Some(present)));
        let explicit_undefined_mask = [3_u64, 0x010200];
        let explicit_undefined_handle = [data.as_ptr().cast::<u8>(), explicit_undefined_mask.as_ptr().cast::<u8>()];
        let explicit_undefined = f64::from_bits(explicit_undefined_handle.as_ptr() as usize as u64);
        let (owned_states, states) = array_native_presence(explicit_undefined, 3).unwrap();
        assert_eq!(unsafe { [states.add(8).read(), states.add(9).read(), states.add(10).read()] }, [0, 2, 1]);
        assert!(owned_states.is_some());
        assert!(array_index_present(explicit_undefined, 1));
        let previous_search = ARRAY_SEARCH.with(|search_slot| search_slot.replace(Some(search)));
        let previous_format = ARRAY_FORMAT.with(|format_slot| format_slot.replace(Some(format)));
        let previous_slice = ARRAY_SLICE.with(|slice| slice.replace(Some(slice_data)));
        let previous_presence_slice = ARRAY_PRESENCE_SLICE.with(|slice| slice.replace(Some(slice_presence)));
        let previous_concat = ARRAY_CONCAT.with(|concat| concat.replace(Some(concat_data)));
        let previous_append = ARRAY_APPEND.with(|append| append.replace(Some(append_data)));
        let previous_reverse = ARRAY_TO_REVERSED.with(|reverse| reverse.replace(Some(reverse_data)));
        let previous_reverse_in_place = ARRAY_REVERSE.with(|reverse| reverse.replace(Some(reverse_in_place)));
        let previous_with = ARRAY_WITH.with(|replace| replace.replace(Some(with_data)));
        let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(Some(allocate)));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let mapped = number_array_map(source, |element, _| element * 2.0);
        assert_ne!(mapped.to_bits() & SPARSE_ARRAY_RESULT_TAG, 0);
        assert_eq!(unsafe { array_data(mapped) }.map(|(_, length)| length), Some(3));
        assert!(array_index_present(mapped, 0));
        assert!(!array_index_present(mapped, 1));
        assert!(array_index_present(mapped, 2));
        let (output, _) = unsafe { array_data(mapped) }.unwrap();
        assert_eq!(unsafe { output.add(8).cast::<f64>().read_unaligned() }, 20.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read_unaligned() }, 60.0);
        let previous_present = CALL_PRESENT.with(|present| present.replace(true));
        assert_eq!(number_array_get(mapped, 1.0), 0.0);
        assert_eq!(take_present(), 0.0);
        assert_eq!(number_array_at(mapped, -2.0), 0.0);
        assert_eq!(take_present(), 0.0);
        assert_eq!(number_array_get(mapped, 2.0), 60.0);
        assert_eq!(take_present(), 1.0);
        assert_eq!(number_array_index_of(mapped, 60.0, 0.0), 2.0);
        assert_eq!(number_array_index_of(source, 30.0, 0.0), 2.0);
        for source_array in [mapped, source] {
            let sliced = array_slice(source_array, 0.0, 2.0);
            assert_ne!(sliced.to_bits() & SPARSE_ARRAY_RESULT_TAG, 0);
            assert!(array_index_present(sliced, 0));
            assert!(!array_index_present(sliced, 1));
            assert_eq!(unsafe { array_data(sliced) }.map(|(_, length)| length), Some(2));
            let reversed = array_to_reversed(sliced);
            assert_ne!(reversed.to_bits() & SPARSE_ARRAY_RESULT_TAG, 0);
            assert!(!array_index_present(reversed, 0));
            assert!(array_index_present(reversed, 1));
            assert_eq!(unsafe { array_data(reversed) }.map(|(_, length)| length), Some(2));
            let replaced = number_array_with(sliced, 1.0, 40.0);
            assert_ne!(replaced.to_bits() & SPARSE_ARRAY_RESULT_TAG, 0);
            assert!(array_index_present(replaced, 0));
            assert!(array_index_present(replaced, 1));
            assert_eq!(unsafe { array_data(replaced) }.map(|(_, length)| length), Some(2));
            let appended = number_array_append(sliced, 40.0);
            assert_ne!(appended.to_bits() & SPARSE_ARRAY_RESULT_TAG, 0);
            assert_eq!(unsafe { array_data(appended) }.map(|(_, length)| length), Some(3));
            assert!(array_index_present(appended, 0));
            assert!(!array_index_present(appended, 1));
            assert!(array_index_present(appended, 2));
            assert_eq!(array_reverse(sliced).to_bits(), sliced.to_bits());
            assert!(!array_index_present(sliced, 0));
            assert!(array_index_present(sliced, 1));
            let sliced_handle = (sliced.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
            unsafe {
                let appended_handle = (appended.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
                let replaced_handle = (replaced.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
                let reversed_handle = (reversed.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
                libc::free(appended_handle.add(1).read().cast());
                libc::free(appended_handle.read().cast());
                libc::free(appended_handle.cast());
                libc::free(replaced_handle.add(1).read().cast());
                libc::free(replaced_handle.read().cast());
                libc::free(replaced_handle.cast());
                libc::free(reversed_handle.add(1).read().cast());
                libc::free(reversed_handle.read().cast());
                libc::free(reversed_handle.cast());
                libc::free(sliced_handle.add(1).read().cast());
                libc::free(sliced_handle.read().cast());
                libc::free(sliced_handle.cast());
            }
        }
        let mut external_data = [2_u64, 10.0_f64.to_bits(), 20.0_f64.to_bits()];
        let mut external_mask = [2_u64, 0x0001];
        let mut external_handle = [external_data.as_mut_ptr().cast::<u8>(), external_mask.as_mut_ptr().cast::<u8>()];
        let external = f64::from_bits(external_handle.as_mut_ptr() as usize as u64);
        assert_eq!(array_reverse(external).to_bits(), external.to_bits());
        assert!(!array_index_present(external, 0));
        assert!(array_index_present(external, 1));
        let new_external_mask = external_handle[1];
        assert_ne!(new_external_mask, external_mask.as_mut_ptr().cast::<u8>());
        unsafe { libc::free(new_external_mask.cast()) };
        let concatenated = array_concat(mapped, source);
        assert_ne!(concatenated.to_bits() & SPARSE_ARRAY_RESULT_TAG, 0);
        assert_eq!(unsafe { array_data(concatenated) }.map(|(_, length)| length), Some(6));
        for index in 0..6 {
            assert_eq!(array_index_present(concatenated, index), index % 3 != 1);
        }
        for array in [mapped, source] {
            let formatted = number_array_join(array,
                f64::from_bits(c",".as_ptr() as usize as u64));
            assert_eq!(unsafe {
                CStr::from_ptr(formatted.to_bits() as usize as *const c_char).to_bytes()
            }, b"masked");
        }
        CALL_PRESENT.with(|present| present.set(previous_present));
        assert!(CALL_ERROR.with(|error| error.get().is_null()));
        let mapped_handle = (mapped.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
        unsafe {
            let concat_handle = (concatenated.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
            libc::free(concat_handle.add(1).read().cast());
            libc::free(concat_handle.read().cast());
            libc::free(concat_handle.cast());
            libc::free(mapped_handle.add(1).read().cast());
            libc::free(mapped_handle.read().cast());
            libc::free(mapped_handle.cast());
        }
        CALL_ERROR.with(|error| error.set(previous_error));
        ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
        ARRAY_INDEX_PRESENT.with(|query| query.set(previous_query));
        ARRAY_SEARCH.with(|search_slot| search_slot.set(previous_search));
        ARRAY_FORMAT.with(|format_slot| format_slot.set(previous_format));
        ARRAY_SLICE.with(|slice| slice.set(previous_slice));
        ARRAY_PRESENCE_SLICE.with(|slice| slice.set(previous_presence_slice));
        ARRAY_CONCAT.with(|concat| concat.set(previous_concat));
        ARRAY_APPEND.with(|append| append.set(previous_append));
        ARRAY_TO_REVERSED.with(|reverse| reverse.set(previous_reverse));
        ARRAY_REVERSE.with(|reverse| reverse.set(previous_reverse_in_place));
        ARRAY_WITH.with(|replace| replace.set(previous_with));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn copy_within_moves_sparse_presence_with_the_data() {
        unsafe extern "C" fn allocate(size: usize, _align: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn copy(
            data: *mut u8, width: usize, target: f64, start: f64, end: f64,
        ) -> *mut u8 {
            assert_eq!((width, target, start, end), (8, 1.0, 0.0, 2.0));
            unsafe { std::ptr::copy(data.add(8), data.add(16), 16) };
            data
        }
        unsafe extern "C" fn fill(
            operation: u8, data: *mut u8, value: f64, start: f64, end: f64,
        ) -> *mut u8 {
            assert_eq!((operation, value, start, end), (0, 40.0, 2.0, 3.0));
            unsafe { data.add(24).cast::<f64>().write_unaligned(value) };
            data
        }
        let mut data = [3_u64, 10.0_f64.to_bits(), 0, 30.0_f64.to_bits()];
        let mut mask = [3_u64, 0x010001];
        let mut handle = [data.as_mut_ptr().cast::<u8>(), mask.as_mut_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_mut_ptr() as usize as u64);
        let old_query = ARRAY_INDEX_PRESENT.with(|slot| slot.replace(Some(present)));
        let old_copy = ARRAY_COPY_WITHIN.with(|slot| slot.replace(Some(copy)));
        let old_fill = ARRAY_FILL.with(|slot| slot.replace(Some(fill)));
        let old_allocate = ARENA_ALLOC.with(|slot| slot.replace(Some(allocate)));
        let old_error = CALL_ERROR.with(|slot| slot.replace(std::ptr::null()));
        assert_eq!(array_copy_within(value, 1.0, 0.0, 2.0).to_bits(), value.to_bits());
        assert_eq!(unsafe { [handle[1].add(8).read(), handle[1].add(9).read(), handle[1].add(10).read()] }, [1, 1, 0]);
        assert_eq!(data[2], 10.0_f64.to_bits());
        let copied_mask = handle[1];
        assert_eq!(array_fill(0, value, 40.0, 2.0, 3.0).to_bits(), value.to_bits());
        assert_eq!(unsafe { [handle[1].add(8).read(), handle[1].add(9).read(), handle[1].add(10).read()] }, [1, 1, 1]);
        assert_eq!(data[3], 40.0_f64.to_bits());
        assert!(CALL_ERROR.with(|slot| slot.get().is_null()));
        unsafe { libc::free(copied_mask.cast()); libc::free(handle[1].cast()) };
        CALL_ERROR.with(|slot| slot.set(old_error));
        ARENA_ALLOC.with(|slot| slot.set(old_allocate));
        ARRAY_COPY_WITHIN.with(|slot| slot.set(old_copy));
        ARRAY_FILL.with(|slot| slot.set(old_fill));
        ARRAY_INDEX_PRESENT.with(|slot| slot.set(old_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn sparse_sort_orders_values_then_undefined_then_holes() {
        unsafe extern "C" fn allocate(size: usize, _align: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn sort(operation: u8, data: *mut u8) -> *mut u8 {
            assert_eq!(operation, 0);
            assert_eq!(unsafe { data.cast::<u64>().read() }, 2);
            unsafe { std::ptr::swap(data.add(8).cast::<u64>(), data.add(16).cast::<u64>()) };
            data
        }
        unsafe extern "C" fn unused_to_sorted(_operation: u8, _data: *const u8) -> *mut u8 {
            panic!("sparse sort must compact before the native sorter")
        }
        let mut data = [4_u64, 10.0_f64.to_bits(), 99, 88, 20.0_f64.to_bits()];
        let mut mask = [4_u64, 0x01020001];
        let mut handle = [data.as_mut_ptr().cast::<u8>(), mask.as_mut_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_mut_ptr() as usize as u64);
        let old_query = ARRAY_INDEX_PRESENT.with(|slot| slot.replace(Some(present)));
        let old_sort = ARRAY_SORT.with(|slot| slot.replace(Some(sort)));
        let old_to_sorted = ARRAY_TO_SORTED.with(|slot| slot.replace(Some(unused_to_sorted)));
        let old_allocate = ARENA_ALLOC.with(|slot| slot.replace(Some(allocate)));
        let old_error = CALL_ERROR.with(|slot| slot.replace(std::ptr::null()));
        let sorted = number_array_to_sorted(value);
        assert_eq!([array_index_state(sorted, 0), array_index_state(sorted, 1),
            array_index_state(sorted, 2), array_index_state(sorted, 3)], [1, 1, 2, 0]);
        let (sorted_data, _) = unsafe { array_data(sorted) }.unwrap();
        assert_eq!(unsafe { sorted_data.add(8).cast::<f64>().read_unaligned() }, 20.0);
        assert_eq!(unsafe { sorted_data.add(16).cast::<f64>().read_unaligned() }, 10.0);
        assert_eq!(data[1], 10.0_f64.to_bits());
        assert_eq!(number_array_sort(value).to_bits(), value.to_bits());
        assert_eq!(data[1], 20.0_f64.to_bits());
        assert_eq!(unsafe { [handle[1].add(8).read(), handle[1].add(9).read(),
            handle[1].add(10).read(), handle[1].add(11).read()] }, [1, 1, 2, 0]);
        unsafe {
            let sorted_handle = (sorted.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
            libc::free(sorted_handle.add(1).read().cast());
            libc::free(sorted_handle.read().cast());
            libc::free(sorted_handle.cast());
            libc::free(handle[1].cast());
        }
        assert!(CALL_ERROR.with(|slot| slot.get().is_null()));
        CALL_ERROR.with(|slot| slot.set(old_error));
        ARENA_ALLOC.with(|slot| slot.set(old_allocate));
        ARRAY_TO_SORTED.with(|slot| slot.set(old_to_sorted));
        ARRAY_SORT.with(|slot| slot.set(old_sort));
        ARRAY_INDEX_PRESENT.with(|slot| slot.set(old_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn sparse_push_and_unshift_keep_one_live_handle_and_states() {
        unsafe extern "C" fn allocate(size: usize, _align: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            let length = unsafe { mask.cast::<u64>().read() as usize };
            if index >= length { 1 } else { unsafe { mask.add(8 + index).read() } }
        }
        unsafe extern "C" fn push(operation: u8, handle: *mut *mut u8, value: f64) -> f64 {
            assert_eq!((operation, value), (0, 40.0));
            let old = unsafe { handle.read() };
            let length = unsafe { old.cast::<u64>().read() as usize };
            let new = unsafe { allocate(8 + (length + 1) * 8, 8) };
            unsafe {
                std::ptr::copy_nonoverlapping(old, new, 8 + length * 8);
                new.cast::<u64>().write((length + 1) as u64);
                new.add(8 + length * 8).cast::<f64>().write_unaligned(value);
                handle.write(new);
            }
            (length + 1) as f64
        }
        unsafe extern "C" fn unshift(operation: u8, handle: *mut *mut u8, value: f64) -> f64 {
            assert_eq!((operation, value), (0, 50.0));
            let old = unsafe { handle.read() };
            let length = unsafe { old.cast::<u64>().read() as usize };
            let new = unsafe { allocate(8 + (length + 1) * 8, 8) };
            unsafe {
                new.cast::<u64>().write((length + 1) as u64);
                new.add(8).cast::<f64>().write_unaligned(value);
                std::ptr::copy_nonoverlapping(old.add(8), new.add(16), length * 8);
                handle.write(new);
            }
            (length + 1) as f64
        }
        let initial = unsafe { allocate(32, 8) };
        unsafe {
            initial.cast::<u64>().write(3);
            initial.add(8).cast::<f64>().write_unaligned(10.0);
            std::ptr::write_bytes(initial.add(16), 0, 16);
        }
        let mut mask = [3_u64, 0x020001];
        let mut handle = [initial, mask.as_mut_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_mut_ptr() as usize as u64);
        let old_query = ARRAY_INDEX_PRESENT.with(|slot| slot.replace(Some(present)));
        let old_push = ARRAY_PUSH.with(|slot| slot.replace(Some(push)));
        let old_unshift = ARRAY_UNSHIFT.with(|slot| slot.replace(Some(unshift)));
        let old_allocate = ARENA_ALLOC.with(|slot| slot.replace(Some(allocate)));
        let old_error = CALL_ERROR.with(|slot| slot.replace(std::ptr::null()));
        assert_eq!(array_push(0, value, 40.0), 4.0);
        let pushed_data = handle[0];
        let pushed_mask = handle[1];
        assert_eq!((0..4).map(|index| array_index_state(value, index)).collect::<Vec<_>>(),
            [1, 0, 2, 1]);
        assert_eq!(array_unshift(0, value, 50.0), 5.0);
        assert_eq!((0..5).map(|index| array_index_state(value, index)).collect::<Vec<_>>(),
            [1, 1, 0, 2, 1]);
        assert_eq!(unsafe { handle[0].add(8).cast::<f64>().read_unaligned() }, 50.0);
        assert!(CALL_ERROR.with(|slot| slot.get().is_null()));
        unsafe {
            libc::free(initial.cast());
            libc::free(pushed_data.cast());
            libc::free(handle[0].cast());
            libc::free(pushed_mask.cast());
            libc::free(handle[1].cast());
        }
        CALL_ERROR.with(|slot| slot.set(old_error));
        ARENA_ALLOC.with(|slot| slot.set(old_allocate));
        ARRAY_UNSHIFT.with(|slot| slot.set(old_unshift));
        ARRAY_PUSH.with(|slot| slot.set(old_push));
        ARRAY_INDEX_PRESENT.with(|slot| slot.set(old_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn sparse_pop_returns_undefined_without_exposing_placeholder() {
        unsafe extern "C" fn allocate(size: usize, _align: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn remove(
            operation: u8, handle: *mut *mut u8, out: *mut f64,
        ) -> i8 {
            assert_eq!(operation, 0);
            let current = unsafe { handle.read() };
            let next = unsafe { allocate(24, 8) };
            unsafe {
                std::ptr::copy_nonoverlapping(current, next, 24);
                next.cast::<u64>().write(2);
                out.write(99.0);
                handle.write(next);
            }
            1
        }
        let mut data = [3_u64, 10.0_f64.to_bits(), 0, 99.0_f64.to_bits()];
        let mut mask = [3_u64, 0x020001];
        let mut handle = [data.as_mut_ptr().cast::<u8>(), mask.as_mut_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_mut_ptr() as usize as u64);
        let old_query = ARRAY_INDEX_PRESENT.with(|slot| slot.replace(Some(present)));
        let old_remove = ARRAY_REMOVE.with(|slot| slot.replace(Some(remove)));
        let old_allocate = ARENA_ALLOC.with(|slot| slot.replace(Some(allocate)));
        let old_error = CALL_ERROR.with(|slot| slot.replace(std::ptr::null()));
        let old_present = CALL_PRESENT.with(|slot| slot.replace(true));
        assert_eq!(array_remove(0, value), 0.0);
        assert!(!CALL_PRESENT.with(Cell::get));
        assert_eq!([array_index_state(value, 0), array_index_state(value, 1)], [1, 0]);
        assert_eq!(unsafe { handle[0].cast::<u64>().read() }, 2);
        assert!(CALL_ERROR.with(|slot| slot.get().is_null()));
        unsafe { libc::free(handle[0].cast()); libc::free(handle[1].cast()) };
        CALL_PRESENT.with(|slot| slot.set(old_present));
        CALL_ERROR.with(|slot| slot.set(old_error));
        ARENA_ALLOC.with(|slot| slot.set(old_allocate));
        ARRAY_REMOVE.with(|slot| slot.set(old_remove));
        ARRAY_INDEX_PRESENT.with(|slot| slot.set(old_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn sparse_splice_preserves_receiver_removed_and_copy_masks() {
        unsafe extern "C" fn allocate(size: usize, _align: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn splice(
            handle: *mut *mut u8, start: f64, count: f64, inserts: *const u8,
        ) -> *mut u8 {
            assert_eq!((start, count), (1.0, 2.0));
            let source = unsafe { handle.read() };
            assert_eq!(unsafe { source.cast::<u64>().read() }, 4);
            assert_eq!(unsafe { inserts.cast::<u64>().read() }, 2);
            let output = unsafe { allocate(40, 8) };
            let removed = unsafe { allocate(24, 8) };
            unsafe {
                output.cast::<u64>().write(4);
                std::ptr::copy_nonoverlapping(source.add(8), output.add(8), 8);
                std::ptr::copy_nonoverlapping(inserts.add(8), output.add(16), 16);
                std::ptr::copy_nonoverlapping(source.add(32), output.add(32), 8);
                removed.cast::<u64>().write(2);
                std::ptr::copy_nonoverlapping(source.add(16), removed.add(8), 16);
                handle.write(output);
            }
            removed
        }
        let mut data = [4_u64, 10.0_f64.to_bits(), 99, 88, 40.0_f64.to_bits()];
        let mut mask = [4_u64, 0x01020001];
        let mut handle = [data.as_mut_ptr().cast::<u8>(), mask.as_mut_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_mut_ptr() as usize as u64);
        let inserts_data = [2_u64, 77, 30.0_f64.to_bits()];
        let inserts_mask = [2_u64, 0x0102];
        let inserts_handle = [inserts_data.as_ptr().cast::<u8>(), inserts_mask.as_ptr().cast::<u8>()];
        let inserts = f64::from_bits(inserts_handle.as_ptr() as usize as u64);
        let old_query = ARRAY_INDEX_PRESENT.with(|slot| slot.replace(Some(present)));
        let old_splice = ARRAY_SPLICE.with(|slot| slot.replace(Some(splice)));
        let old_allocate = ARENA_ALLOC.with(|slot| slot.replace(Some(allocate)));
        let old_error = CALL_ERROR.with(|slot| slot.replace(std::ptr::null()));
        let copied = array_to_spliced(value, 1.0, 2.0, inserts);
        assert_eq!((0..4).map(|index| array_index_state(copied, index)).collect::<Vec<_>>(),
            [1, 2, 1, 1]);
        assert_eq!(handle[0], data.as_mut_ptr().cast::<u8>());
        let removed = array_splice(value, 1.0, 2.0, inserts);
        assert_eq!((0..2).map(|index| array_index_state(removed, index)).collect::<Vec<_>>(),
            [0, 2]);
        assert_eq!((0..4).map(|index| array_index_state(value, index)).collect::<Vec<_>>(),
            [1, 2, 1, 1]);
        assert!(CALL_ERROR.with(|slot| slot.get().is_null()));
        unsafe {
            for result in [copied, removed] {
                let result_handle = (result.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
                libc::free(result_handle.add(1).read().cast());
                libc::free(result_handle.read().cast());
                libc::free(result_handle.cast());
            }
            libc::free(handle[0].cast());
            libc::free(handle[1].cast());
        }
        CALL_ERROR.with(|slot| slot.set(old_error));
        ARENA_ALLOC.with(|slot| slot.set(old_allocate));
        ARRAY_SPLICE.with(|slot| slot.set(old_splice));
        ARRAY_INDEX_PRESENT.with(|slot| slot.set(old_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn sparse_set_marks_written_hole_and_retains_new_gap() {
        unsafe extern "C" fn allocate(size: usize, _align: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn set(operation: u8, handle: *mut *mut u8, index: f64, value: f64) -> i8 {
            assert_eq!(operation, 0);
            let current = unsafe { handle.read() };
            let length = unsafe { current.cast::<u64>().read() as usize };
            let index = index as usize;
            if index < length {
                unsafe { current.add(8 + index * 8).cast::<u64>().write(value.to_bits()) };
            } else {
                let next = unsafe { allocate(8 + (index + 1) * 8, 8) };
                unsafe {
                    next.cast::<u64>().write((index + 1) as u64);
                    std::ptr::copy_nonoverlapping(current.add(8), next.add(8), length * 8);
                    next.add(8 + length * 8).write_bytes(0, (index - length) * 8);
                    next.add(8 + index * 8).cast::<u64>().write(value.to_bits());
                    handle.write(next);
                }
            }
            1
        }
        let mut data = [3_u64, 10.0_f64.to_bits(), 0, 0];
        let mut mask = [3_u64, 0x020001];
        let mut handle = [data.as_mut_ptr().cast::<u8>(), mask.as_mut_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_mut_ptr() as usize as u64);
        let old_query = ARRAY_INDEX_PRESENT.with(|slot| slot.replace(Some(present)));
        let old_set = ARRAY_SET.with(|slot| slot.replace(Some(set)));
        let old_allocate = ARENA_ALLOC.with(|slot| slot.replace(Some(allocate)));
        let old_error = CALL_ERROR.with(|slot| slot.replace(std::ptr::null()));
        assert_eq!(array_set(0, value, 1.0, 20.0), 20.0);
        assert_eq!(array_set(0, value, 4.0, 50.0), 50.0);
        assert_eq!((0..5).map(|index| array_index_state(value, index)).collect::<Vec<_>>(),
            [1, 1, 2, 0, 1]);
        assert_eq!(unsafe { handle[0].cast::<u64>().read() }, 5);
        assert!(CALL_ERROR.with(|slot| slot.get().is_null()));
        unsafe { libc::free(handle[0].cast()); libc::free(handle[1].cast()) };
        CALL_ERROR.with(|slot| slot.set(old_error));
        ARENA_ALLOC.with(|slot| slot.set(old_allocate));
        ARRAY_SET.with(|slot| slot.set(old_set));
        ARRAY_INDEX_PRESENT.with(|slot| slot.set(old_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn array_scan_queries_sparse_native_handle_without_changing_dense_legacy_abi() {
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        let data = [3_u64, 10.0_f64.to_bits(), 0, 30.0_f64.to_bits()];
        let mask = [3_u64, 0x010001];
        let handle = [data.as_ptr().cast::<u8>(), mask.as_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_ptr() as usize as u64);
        let previous_query = ARRAY_INDEX_PRESENT.with(|query| query.replace(Some(present)));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let visited = std::cell::Cell::new(0_u8);
        let matched = primitive_array_scan(value, 0, 0, |_, index, _| {
            visited.set(visited.get() | (1 << index));
            false
        });
        assert_eq!((matched, visited.get()), (0.0, 0b101));
        ARRAY_INDEX_PRESENT.with(|query| query.set(None));
        let dense_handle = data.as_ptr().cast::<u8>();
        assert!(array_index_present(f64::from_bits(&dense_handle as *const _ as usize as u64), 1));
        ARRAY_INDEX_PRESENT.with(|query| query.set(previous_query));
        CALL_ERROR.with(|error| error.set(previous_error));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn explicit_undefined_scan_result_keeps_tag_and_filtered_presence() {
        unsafe extern "C" fn allocate(size: usize, _align: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        let data = [2_u64, 10.0_f64.to_bits(), 99.0_f64.to_bits()];
        let mut mask = [2_u64, 0x0201];
        let handle = [data.as_ptr().cast::<u8>(), mask.as_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_ptr() as usize as u64);
        let old_query = ARRAY_INDEX_PRESENT.with(|slot| slot.replace(Some(present)));
        let old_allocate = ARENA_ALLOC.with(|slot| slot.replace(Some(allocate)));
        let old_error = CALL_ERROR.with(|slot| slot.replace(std::ptr::null()));
        let old_present = CALL_PRESENT.with(|slot| slot.replace(true));
        assert_eq!(array_index_state(value, 1), 2);
        let found = primitive_array_scan(value, 0, 2, |element, index, state| {
            if index == 1 { assert_eq!((element, state), (None, 2)); }
            if index == 1 { mask[1] = 0x0101; }
            index == 1
        });
        assert_eq!(found, 0.0);
        let tagged = dynamic_array_scan_result(DYNAMIC_NUMBER_ARRAY_TAG, 2, found);
        assert_eq!(dynamic_tag(tagged), DYNAMIC_UNDEFINED_TAG as f64);
        mask[1] = 0x0201;
        CALL_PRESENT.with(|slot| slot.set(true));
        let filtered = primitive_array_scan(value, 0, 6, |_, index, _| index == 1);
        assert_eq!(array_index_state(filtered, 0), 2);
        let filtered_handle = (filtered.to_bits() & !SPARSE_ARRAY_RESULT_TAG) as usize as *mut *mut u8;
        unsafe {
            libc::free(filtered_handle.add(1).read().cast());
            libc::free(filtered_handle.read().cast());
            libc::free(filtered_handle.cast());
            libc::free((tagged.to_bits() as usize as *mut u8).cast());
        }
        DYNAMIC_VALUES.with(|values| { values.borrow_mut().remove(&(tagged.to_bits() as usize)); });
        assert!(CALL_ERROR.with(|slot| slot.get().is_null()));
        CALL_PRESENT.with(|slot| slot.set(old_present));
        CALL_ERROR.with(|slot| slot.set(old_error));
        ARENA_ALLOC.with(|slot| slot.set(old_allocate));
        ARRAY_INDEX_PRESENT.with(|slot| slot.set(old_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn array_scan_keeps_selected_value_and_refreshes_truncated_handle() {
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        let original = [3_u64, 10.0_f64.to_bits(), 20.0_f64.to_bits(), 30.0_f64.to_bits()];
        let replacement = [1_u64, 99.0_f64.to_bits()];
        let mut handle = original.as_ptr().cast::<u8>();
        let slot = &mut handle as *mut *const u8;
        let value = f64::from_bits(slot as usize as u64);
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let previous_present = CALL_PRESENT.with(|present| present.replace(true));
        let found = primitive_array_scan(value, 0, 2, |_, index, _| {
            assert_eq!(index, 0);
            unsafe { slot.write(replacement.as_ptr().cast()) };
            true
        });
        assert_eq!(found, 10.0);
        unsafe { slot.write(original.as_ptr().cast()) };
        let visits = std::cell::Cell::new(0);
        let matched = primitive_array_scan(value, 0, 0, |_, index, _| {
            assert_eq!(index, 0);
            visits.set(visits.get() + 1);
            unsafe { slot.write(replacement.as_ptr().cast()) };
            false
        });
        assert_eq!((matched, visits.get()), (0.0, 1));
        unsafe { slot.write(original.as_ptr().cast()) };
        let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(Some(allocate)));
        let filtered = primitive_array_scan(value, 0, 6, |element, index, _| {
            assert_eq!((element, index), (Some(10.0), 0));
            unsafe { slot.write(replacement.as_ptr().cast()) };
            true
        });
        let output = (filtered.to_bits() & !ARRAY_RESULT_TAG) as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        unsafe { libc::free(output.cast()) };
        ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
        CALL_ERROR.with(|error| error.set(previous_error));
        CALL_PRESENT.with(|present| present.set(previous_present));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn array_find_visits_hole_as_distinct_undefined_member() {
        let data = [2_u64, 10.0_f64.to_bits(), 123.0_f64.to_bits()];
        let mask = [2_u64, 1];
        let handle = [data.as_ptr().cast::<u8>(), mask.as_ptr().cast::<u8>()];
        let value = f64::from_bits(handle.as_ptr() as usize as u64 | SPARSE_ARRAY_RESULT_TAG);
        let previous_present = CALL_PRESENT.with(|present| present.replace(true));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let observed = std::cell::Cell::new(false);
        let index = primitive_array_scan(value, 0, 3, |element, position, _| {
            if position == 1 { observed.set(element.is_none()); }
            element.is_none()
        });
        assert_eq!(index, 1.0);
        assert!(observed.get());
        let found = primitive_array_scan(value, 0, 2, |element, _, _| element.is_none());
        assert_eq!(found, 0.0);
        assert!(!CALL_PRESENT.with(Cell::get));
        assert!(CALL_ERROR.with(|error| error.get().is_null()));
        CALL_ERROR.with(|error| error.set(previous_error));
        CALL_PRESENT.with(|present| present.set(previous_present));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn dynamic_jit_find_index_receives_undefined_tag_for_hole() {
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        let data = [2_u64, 10.0_f64.to_bits(), 99.0_f64.to_bits()];
        let mask = [2_u64, 1];
        let handle = [data.as_ptr().cast::<u8>(), mask.as_ptr().cast::<u8>()];
        let previous_query = ARRAY_INDEX_PRESENT.with(|query| query.replace(Some(present)));
        let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(Some(allocate)));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let array = arena_dynamic(
            DYNAMIC_NUMBER_ARRAY_TAG,
            handle.as_ptr() as usize as u64,
        );
        let callback = CString::new("u0n,tagkind,c4028000000000000,==,cbresultboolean").unwrap();
        let index = dynamic_array_jit_scan_impl(
            array,
            f64::from_bits(callback.as_ptr() as usize as u64),
            3.0,
            None,
        );
        assert_eq!(index, 1.0);
        assert_eq!(primitive_array_jit_scan(
            f64::from_bits(handle.as_ptr() as usize as u64),
            f64::from_bits(callback.as_ptr() as usize as u64),
            3.0,
        ), 1.0);
        let empty_captures = [0_u64];
        let capture_handle = [empty_captures.as_ptr().cast::<u8>()];
        assert_eq!(primitive_array_jit_scan_captured(
            f64::from_bits(handle.as_ptr() as usize as u64),
            f64::from_bits(callback.as_ptr() as usize as u64),
            f64::from_bits(capture_handle.as_ptr() as usize as u64),
            3.0,
        ), 1.0);
        let explicit_undefined_mask = [2_u64, 0x0201];
        let explicit_handle = [data.as_ptr().cast::<u8>(), explicit_undefined_mask.as_ptr().cast::<u8>()];
        let mapped = primitive_array_jit_map(
            f64::from_bits(explicit_handle.as_ptr() as usize as u64),
            f64::from_bits(callback.as_ptr() as usize as u64),
            0.0,
        );
        let (mapped_data, mapped_length) = unsafe { array_data(mapped) }.unwrap();
        assert_eq!(mapped_length, 2);
        assert_eq!(unsafe { mapped_data.add(8).cast::<f64>().read_unaligned() }, 0.0);
        assert_eq!(unsafe { mapped_data.add(16).cast::<f64>().read_unaligned() }, 1.0);
        unsafe { libc::free(mapped_data.cast_mut().cast()) };
        let undefined = dynamic_from_parts(DYNAMIC_UNDEFINED_TAG as f64, 0.0);
        assert_eq!(dynamic_array_compare(array, undefined, 28.0), 1.0);
        assert_eq!(dynamic_array_compare(array, undefined, 30.0), 1.0);
        assert_eq!(dynamic_array_compare(array, undefined, 29.0), 0.0);
        assert_eq!(dynamic_array_includes(array, undefined, 0.0), 1.0);
        assert_eq!(dynamic_array_includes(array, undefined, 2.0), 0.0);
        assert_eq!(dynamic_array_index_of(array, undefined, 0.0), -1.0);
        assert_eq!(dynamic_array_last_index_of(array, undefined, 0.0), -1.0);
        assert!(CALL_ERROR.with(|error| error.get().is_null()));
        for value in [array, undefined] {
            DYNAMIC_VALUES.with(|values| { values.borrow_mut().remove(&(value.to_bits() as usize)); });
            unsafe { libc::free((value.to_bits() as usize as *mut u8).cast()) };
        }
        CALL_ERROR.with(|error| error.set(previous_error));
        ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
        ARRAY_INDEX_PRESENT.with(|query| query.set(previous_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn dynamic_jit_reduce_preserves_sparse_undefined_inputs() {
        unsafe extern "C" fn present(handle: *const *const u8, index: usize) -> u8 {
            let mask = unsafe { handle.add(1).read() };
            unsafe { mask.add(8 + index).read() }
        }
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        let data = [3_u64, 77.0_f64.to_bits(), 5.0_f64.to_bits(), 9.0_f64.to_bits()];
        let mask = [3_u64, 0x010200];
        let handle = [data.as_ptr().cast::<u8>(), mask.as_ptr().cast::<u8>()];
        let previous_query = ARRAY_INDEX_PRESENT.with(|query| query.replace(Some(present)));
        let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(Some(allocate)));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let prior_values = DYNAMIC_VALUES.with(|values| values.borrow().clone());
        let array = arena_dynamic(DYNAMIC_NUMBER_ARRAY_TAG, handle.as_ptr() as usize as u64);
        let callback = CString::new(
            "a0,u1n,tagkind,c4028000000000000,==,+,cbresultnumber",
        ).unwrap();
        let callback = f64::from_bits(callback.as_ptr() as usize as u64);
        assert_eq!(dynamic_array_jit_reduce(
            array, 0.0, callback, false, None,
        ), 1.0);

        let callback = CString::new(
            "u0n,tagkind,c4028000000000000,==,if,u2n,dynnum,else,u0n,dynnum,u2n,dynnum,+,end,tagnum,cbresultdynamic",
        ).unwrap();
        let result = dynamic_array_jit_reduce_unseeded(
            array, f64::from_bits(callback.as_ptr() as usize as u64), false, None,
        );
        assert_eq!(dynamic_to_number(result), 9.0);
        assert!(CALL_ERROR.with(|error| error.get().is_null()));

        let added_values = DYNAMIC_VALUES.with(|values| {
            let mut values = values.borrow_mut();
            let added = values.difference(&prior_values).copied().collect::<Vec<_>>();
            for pointer in added { values.remove(&pointer); }
            added
        });
        for pointer in added_values { unsafe { libc::free(pointer as *mut u8) }; }
        assert!(CALL_ERROR.with(|error| error.get().is_null()));
        CALL_ERROR.with(|error| error.set(previous_error));
        ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
        ARRAY_INDEX_PRESENT.with(|query| query.set(previous_query));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn dynamic_undefined_tag_keeps_primitive_coercion_distinct() {
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        let previous_allocator = ARENA_ALLOC.with(|allocator| allocator.replace(Some(allocate)));
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let undefined = dynamic_from_parts(DYNAMIC_UNDEFINED_TAG as f64, 0.0);
        let number = tag_number(3.0);
        let boxed_string = tag_string(f64::from_bits(c"".as_ptr() as usize as u64));
        assert_eq!(dynamic_tag(undefined), DYNAMIC_UNDEFINED_TAG as f64);
        assert_eq!(dynamic_to_boolean(undefined), 0.0);
        assert!(!jit_callback_truthy(undefined, JitCallbackResultKind::Dynamic));
        // The same registered pointer bits have number semantics when the producer says Number.
        assert!(jit_callback_truthy(
            f64::from_bits(undefined.to_bits()),
            JitCallbackResultKind::Number,
        ));
        assert!(!jit_callback_truthy(f64::NAN, JitCallbackResultKind::Number));
        assert!(!jit_callback_truthy(0.0, JitCallbackResultKind::Boolean));
        assert!(!jit_callback_truthy(
            f64::from_bits(c"".as_ptr() as usize as u64),
            JitCallbackResultKind::String,
        ));
        assert!(jit_callback_truthy(boxed_string, JitCallbackResultKind::Dynamic));
        assert!(dynamic_to_number(undefined).is_nan());
        assert_eq!(unsafe { CStr::from_ptr(type_of_dynamic(undefined).to_bits() as usize as *const c_char) }.to_bytes(), b"undefined");
        assert_eq!(unsafe { CStr::from_ptr(dynamic_to_string(undefined).to_bits() as usize as *const c_char) }.to_bytes(), b"undefined");
        assert_eq!(dynamic_strict_equal(undefined, undefined), 1.0);
        assert_eq!(dynamic_equal(undefined, number), 0.0);
        assert_eq!(dynamic_less(undefined, number), 0.0);
        let sum = dynamic_add(undefined, number);
        assert!(dynamic_to_number(sum).is_nan());
        assert!(CALL_ERROR.with(|error| error.get().is_null()));
        for value in [undefined, number, sum, boxed_string] {
            DYNAMIC_VALUES.with(|values| { values.borrow_mut().remove(&(value.to_bits() as usize)); });
            unsafe { libc::free((value.to_bits() as usize as *mut u8).cast()) };
        }
        CALL_ERROR.with(|error| error.set(previous_error));
        ARENA_ALLOC.with(|allocator| allocator.set(previous_allocator));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn jit_callback_result_kind_comes_from_full_program_analysis() {
        let arithmetic = "a0,c3ff0000000000000,+";
        let arithmetic = NumericProgram::parse(&format!("expr:{arithmetic}:legacy-callback")).unwrap();
        assert_eq!(arithmetic.callback_result_kind_code(), Some(0));

        let conditional = "b0,if,t61,else,t62,end";
        let conditional = NumericProgram::parse(&format!("expr:{conditional}:legacy-callback")).unwrap();
        assert_eq!(conditional.callback_result_kind_code(), Some(2));

        let mismatched = "b0,if,a0,else,t61,end";
        let mismatched = NumericProgram::parse(&format!("expr:{mismatched}:legacy-callback")).unwrap();
        assert_eq!(mismatched.callback_result_kind_code(), None);

        let branch = NumericProgram::parse("expr:b0,if,a0,else,a1,end:explicit-number").unwrap();
        assert_eq!(branch.callback_result_kind_code(), Some(0));
        let boolean = NumericProgram::parse("expr:a0,asbool:to-boolean").unwrap();
        assert_eq!(boolean.callback_result_kind_code(), Some(1));
        let boolean_not = NumericProgram::parse("expr:a0,boolnot:to-boolean").unwrap();
        assert_eq!(boolean_not.callback_result_kind_code(), Some(1));
        let maybe = NumericProgram::parse(
            "expr:rn0,c3ff0000000000000,rnat:maybe-number",
        )
        .unwrap();
        assert_eq!(maybe.callback_result_kind_code(), Some(0));
        assert_eq!(
            proven_callback_result_kind(0, Some(JitCallbackResultKind::String)),
            None,
        );
        assert_eq!(
            proven_callback_result_kind(2, Some(JitCallbackResultKind::String)),
            Some(JitCallbackResultKind::String),
        );
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn dynamic_equality_accepts_presence_aware_scalar_boxing_tokens() {
        let program = NumericProgram::parse(
            "expr:a0,tagmaybenum,a1,tagmaybestr,dynseq:typed-strict-equality",
        )
        .expect("presence-aware tags and strict dynamic comparison parse");
        assert_eq!(program.callback_result_kind(), Some(CallbackValueKind::Boolean));
        assert!(program.machine_code().is_some());
        let mismatched = NumericProgram::parse("expr:a0,tagmaybestr:invalid-tag-proof")
            .expect("the parser retains a well-formed tag instruction");
        assert_eq!(mismatched.callback_result_kind(), None);
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_checks_pointer_consumer_input_family() {
        let number_as_string = NumericProgram::parse("expr:a0,strlen:wrong-pointer-family").unwrap();
        assert_eq!(number_as_string.callback_result_kind(), None);
        let valid_string = NumericProgram::parse("expr:s0,strlen:string-length").unwrap();
        assert_eq!(valid_string.callback_result_kind(), Some(CallbackValueKind::Number));
        let valid_selected_string = NumericProgram::parse(
            "expr:s0,ifpresent,s1,strlen,else,c3ff0000000000000,end:selected-string-length",
        )
        .unwrap();
        assert_eq!(valid_selected_string.callback_result_kind(), Some(CallbackValueKind::Number));

        assert_eq!(valid_string.callback_result_kind_code_for_environment(&[0], &[]), None);
        assert_eq!(valid_string.callback_result_kind_code_for_environment(&[2], &[]), Some(0));
        let paired = NumericProgram::parse("expr:u0nbs,dynstr:dynamic-pair").unwrap();
        assert_eq!(paired.callback_result_kind_code_for_environment(&[3, 9], &[]), None);
        assert_eq!(paired.callback_result_kind_code_for_environment(&[3, 9], &[0]), Some(2));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn strict_mismatch_compares_nullish_presence_and_reference_identity_safely() {
        assert_eq!(strict_presence_mismatch(1, 1), 1.0);
        assert_eq!(strict_presence_mismatch(2, 2), 1.0);
        assert_eq!(strict_presence_mismatch(1, 2), 0.0);
        assert_eq!(strict_presence_mismatch(0, 0), 0.0);
        let maybe = NumericProgram::parse("expr:a0,a1,strictmaybe:optional-strict-mismatch")
            .expect("presence-only strict mismatch parses");
        assert_eq!(maybe.callback_result_kind(), Some(CallbackValueKind::Boolean));
        assert!(maybe.machine_code().is_some());
        let same = NumericProgram::parse("expr:a0,a1,refsame:optional-reference-same-value")
            .expect("presence-aware reference SameValue parses");
        assert!(same.machine_code().is_some());
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn numeric_parser_retains_callback_source_representations() {
        let program = NumericProgram::parse(
            "expr:a0,ls1,ln2,ld3,s4,u0nbs,objs5,objb6,rn7,ds8,h9,rnl10,dnl11,en12,objrn13,objdn14,objoptrn15,c3ff0000000000000,t61:callback-source-kinds",
        )
        .expect("all annotated source forms parse");
        assert_eq!(program.callback_source_kind(0), Some(CallbackSourceKind::Argument(CallbackValueKind::Number)));
        assert_eq!(program.callback_source_kind(1), Some(CallbackSourceKind::Local(CallbackValueKind::String)));
        assert_eq!(program.callback_source_kind(2), Some(CallbackSourceKind::Local(CallbackValueKind::Number)));
        assert_eq!(program.callback_source_kind(3), Some(CallbackSourceKind::Local(CallbackValueKind::Dynamic)));
        assert_eq!(program.callback_source_kind(4), Some(CallbackSourceKind::Argument(CallbackValueKind::String)));
        assert_eq!(program.callback_source_kind(5), Some(CallbackSourceKind::Argument(CallbackValueKind::Dynamic)));
        assert_eq!(program.callback_source_kind(6), Some(CallbackSourceKind::Field(CallbackValueKind::String)));
        assert_eq!(program.callback_source_kind(7), Some(CallbackSourceKind::Field(CallbackValueKind::Boolean)));
        assert_eq!(program.callback_source_kind(8), Some(CallbackSourceKind::Argument(CallbackValueKind::ArrayReference)));
        assert_eq!(program.callback_source_kind(9), Some(CallbackSourceKind::Argument(CallbackValueKind::DictionaryReference)));
        assert_eq!(program.callback_source_kind(10), Some(CallbackSourceKind::Argument(CallbackValueKind::NativeHandle)));
        assert_eq!(program.callback_source_kind(11), Some(CallbackSourceKind::Local(CallbackValueKind::ArrayReference)));
        assert_eq!(program.callback_source_kind(12), Some(CallbackSourceKind::Local(CallbackValueKind::DictionaryReference)));
        assert_eq!(program.callback_source_kind(13), Some(CallbackSourceKind::Argument(CallbackValueKind::Unknown)));
        assert_eq!(program.callback_source_kind(14), Some(CallbackSourceKind::Field(CallbackValueKind::ArrayReference)));
        assert_eq!(program.callback_source_kind(15), Some(CallbackSourceKind::Field(CallbackValueKind::DictionaryReference)));
        assert_eq!(program.callback_source_kind(16), Some(CallbackSourceKind::MaybeField(CallbackValueKind::ArrayReference)));
        assert_eq!(program.callback_source_kind(17), Some(CallbackSourceKind::Constant(CallbackValueKind::Number)));
        assert_eq!(program.callback_source_kind(18), Some(CallbackSourceKind::Constant(CallbackValueKind::String)));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_joins_conditional_local_writes() {
        let same_kind = NumericProgram::parse(
            "expr:b0,if,t61,setl0,else,t62,setl0,end,l0:callback-local-join",
        )
        .expect("same-kind local assignments parse");
        assert_eq!(same_kind.callback_result_kind(), Some(CallbackValueKind::String));

        let conflicting = NumericProgram::parse(
            "expr:b0,if,t61,setl0,else,a0,setl0,end,l0:callback-local-conflict",
        )
        .expect("conflicting local assignments parse");
        assert_eq!(conflicting.callback_result_kind(), None);
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_refines_optional_result_only_in_present_arm() {
        let same_kind = NumericProgram(
            vec![
                NumericValue::Argument(0),
                NumericValue::PrimitiveArrayTruthy(0, 2),
                NumericValue::PresentConditionalStart,
                NumericValue::ConditionalAlternate,
                NumericValue::Constant(0.0),
                NumericValue::ShortCircuitEnd,
            ],
            vec![
                Some(CallbackSourceKind::Argument(CallbackValueKind::ArrayReference)),
                None,
                None,
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                None,
            ],
        );
        assert_eq!(same_kind.callback_result_kind(), Some(CallbackValueKind::Number));

        let different_kind = NumericProgram(
            vec![
                NumericValue::Argument(0),
                NumericValue::PrimitiveArrayTruthy(0, 2),
                NumericValue::PresentConditionalStart,
                NumericValue::ConditionalAlternate,
                NumericValue::Constant(0.0),
                NumericValue::TypeOfNumber,
                NumericValue::ShortCircuitEnd,
            ],
            vec![
                Some(CallbackSourceKind::Argument(CallbackValueKind::ArrayReference)),
                None,
                None,
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                None,
                None,
            ],
        );
        assert_eq!(different_kind.callback_result_kind(), None);
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_joins_short_circuit_value_and_rhs() {
        let same_kind = NumericProgram::parse(
            "expr:t61,b0,||,t62,end:callback-short-circuit-same",
        )
        .expect("short-circuit values parse");
        assert_eq!(same_kind.callback_result_kind(), Some(CallbackValueKind::String));

        let different_kind = NumericProgram::parse(
            "expr:t61,b0,||,a0,end:callback-short-circuit-conflict",
        )
        .expect("mixed short-circuit values parse");
        assert_eq!(different_kind.callback_result_kind(), None);
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_joins_typed_try_and_catch_exits() {
        let number = Some(CallbackSourceKind::Constant(CallbackValueKind::Number));
        let plain = NumericProgram(
            vec![
                NumericValue::TryStart,
                NumericValue::Constant(1.0),
                NumericValue::Constant(2.0),
                NumericValue::Throw,
                NumericValue::CatchStart,
                NumericValue::TryEnd,
            ],
            vec![None, number, number, None, None, None],
        );
        assert_eq!(plain.callback_result_kind(), Some(CallbackValueKind::Number));

        let tagged = NumericProgram(
            vec![
                NumericValue::TaggedTryStart,
                NumericValue::Constant(1.0),
                NumericValue::Constant(2.0),
                NumericValue::TaggedThrow(7),
                NumericValue::CatchStart,
                NumericValue::DropUnder,
                NumericValue::TryEnd,
            ],
            vec![None, number, number, None, None, None, None],
        );
        assert_eq!(tagged.callback_result_kind(), Some(CallbackValueKind::Number));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_keeps_normal_try_exit_when_catch_is_unreachable() {
        let program = NumericProgram(
            vec![
                NumericValue::TryStart,
                NumericValue::Constant(1.0),
                NumericValue::CatchStart,
                NumericValue::StringConstant(0),
                NumericValue::TryEnd,
            ],
            vec![
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::String)),
                None,
            ],
        );
        assert_eq!(program.callback_result_kind(), Some(CallbackValueKind::Number));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_excludes_throw_path_from_normal_branch() {
        let number = Some(CallbackSourceKind::Constant(CallbackValueKind::Number));
        let program = NumericProgram(
            vec![
                NumericValue::TryStart,
                NumericValue::Constant(1.0),
                NumericValue::ConditionalStart,
                NumericValue::Constant(2.0),
                NumericValue::Throw,
                NumericValue::ConditionalAlternate,
                NumericValue::Constant(3.0),
                NumericValue::ShortCircuitEnd,
                NumericValue::CatchStart,
                NumericValue::TryEnd,
            ],
            vec![None, number, None, number, None, None, number, None, None, None],
        );
        assert_eq!(program.callback_result_kind(), Some(CallbackValueKind::Number));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_ignores_dead_local_writes_after_early_return() {
        let program = NumericProgram(
            vec![
                NumericValue::Constant(1.0),
                NumericValue::EarlyReturn,
                NumericValue::StringConstant(0),
                NumericValue::LocalSet(0),
                NumericValue::LocalGet(0),
            ],
            vec![
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::String)),
                None,
                None,
            ],
        );
        assert_eq!(program.callback_result_kind(), Some(CallbackValueKind::Number));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_tracks_loop_break_and_continue_edges() {
        let number = Some(CallbackSourceKind::Constant(CallbackValueKind::Number));
        let breaks = NumericProgram(
            vec![
                NumericValue::LoopStart,
                NumericValue::Constant(1.0),
                NumericValue::LoopWhile,
                NumericValue::Constant(2.0),
                NumericValue::LoopBreak(0),
                NumericValue::StringConstant(0),
                NumericValue::LoopContinuePoint,
                NumericValue::LoopEnd,
                NumericValue::Constant(3.0),
            ],
            vec![None, number, None, number, None, None, None, None, number],
        );
        assert_eq!(breaks.callback_result_kind(), Some(CallbackValueKind::Number));

        let continues = NumericProgram(
            vec![
                NumericValue::LoopStart,
                NumericValue::Constant(1.0),
                NumericValue::LoopWhile,
                NumericValue::LoopContinue(0),
                NumericValue::LoopContinuePoint,
                NumericValue::LoopEnd,
                NumericValue::Constant(4.0),
            ],
            vec![None, number, None, None, None, None, number],
        );
        assert_eq!(continues.callback_result_kind(), Some(CallbackValueKind::Number));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_joins_switch_fallthrough_and_break() {
        let number = Some(CallbackSourceKind::Constant(CallbackValueKind::Number));
        let program = NumericProgram(
            vec![
                NumericValue::Constant(0.0),
                NumericValue::SwitchStart,
                NumericValue::SwitchCaseStart,
                NumericValue::Constant(1.0),
                NumericValue::SwitchCaseBody,
                NumericValue::Constant(2.0),
                NumericValue::LocalSet(0),
                NumericValue::SwitchBreak,
                NumericValue::StringConstant(0),
                NumericValue::LocalSet(0),
                NumericValue::SwitchCaseStart,
                NumericValue::Constant(3.0),
                NumericValue::SwitchCaseBody,
                NumericValue::Constant(4.0),
                NumericValue::LocalSet(0),
                NumericValue::SwitchDefault,
                NumericValue::Constant(5.0),
                NumericValue::LocalSet(0),
                NumericValue::SwitchEnd,
                NumericValue::LocalGet(0),
            ],
            vec![
                number,
                None,
                None,
                number,
                None,
                number,
                None,
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::String)),
                None,
                None,
                number,
                None,
                number,
                None,
                None,
                number,
                None,
                None,
                None,
            ],
        );
        assert_eq!(program.callback_result_kind(), Some(CallbackValueKind::Number));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_replays_default_after_later_case_mutates_dispatch_state() {
        let number = Some(CallbackSourceKind::Constant(CallbackValueKind::Number));
        let string = Some(CallbackSourceKind::Constant(CallbackValueKind::String));
        let program = NumericProgram(
            vec![
                NumericValue::Constant(0.0),
                NumericValue::Constant(0.0),
                NumericValue::SwitchStart,
                NumericValue::SwitchDefault,
                NumericValue::LocalGet(0),
                NumericValue::LocalSet(0),
                NumericValue::SwitchBreak,
                NumericValue::SwitchCaseStart,
                NumericValue::StringConstant(0),
                NumericValue::LocalSet(0),
                NumericValue::Constant(1.0),
                NumericValue::SwitchCaseBody,
                NumericValue::LocalGet(0),
                NumericValue::LocalSet(0),
                NumericValue::SwitchBreak,
                NumericValue::SwitchEnd,
                NumericValue::LocalGet(0),
            ],
            vec![
                number,
                number,
                None,
                None,
                None,
                None,
                None,
                None,
                string,
                None,
                number,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        );
        assert_eq!(program.callback_result_kind(), Some(CallbackValueKind::String));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_switch_break_truncates_temporaries_and_rejects_underflow() {
        let number = Some(CallbackSourceKind::Constant(CallbackValueKind::Number));
        let extra_temporary = NumericProgram(
            vec![
                NumericValue::Constant(0.0),
                NumericValue::SwitchStart,
                NumericValue::SwitchCaseStart,
                NumericValue::Constant(1.0),
                NumericValue::SwitchCaseBody,
                NumericValue::Constant(9.0),
                NumericValue::SwitchBreak,
                NumericValue::SwitchEnd,
                NumericValue::Constant(4.0),
            ],
            vec![number, None, None, number, None, number, None, None, number],
        );
        assert_eq!(extra_temporary.callback_result_kind(), Some(CallbackValueKind::Number));

        let underflow = NumericProgram(
            vec![
                NumericValue::Constant(0.0),
                NumericValue::SwitchStart,
                NumericValue::Drop,
                NumericValue::SwitchBreak,
                NumericValue::SwitchEnd,
                NumericValue::Constant(4.0),
            ],
            vec![number, None, None, None, None, number],
        );
        assert_eq!(underflow.callback_result_kind(), None);
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_tracks_locals_as_overwritten_stack_slots() {
        let overwritten = NumericProgram(
            vec![
                NumericValue::Constant(1.0),
                NumericValue::StringConstant(0),
                NumericValue::LocalSet(0),
            ],
            vec![
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                Some(CallbackSourceKind::Constant(CallbackValueKind::String)),
                None,
            ],
        );
        assert_eq!(overwritten.callback_result_kind(), Some(CallbackValueKind::String));

        let mismatched_read = NumericProgram(
            vec![NumericValue::Constant(1.0), NumericValue::LocalGet(0)],
            vec![
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                Some(CallbackSourceKind::Local(CallbackValueKind::String)),
            ],
        );
        assert_eq!(mismatched_read.callback_result_kind(), None);
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_preserves_mutated_prefixes_on_result_and_catch_edges() {
        let number = Some(CallbackSourceKind::Constant(CallbackValueKind::Number));
        let string = Some(CallbackSourceKind::Constant(CallbackValueKind::String));
        let result = NumericProgram(
            vec![
                NumericValue::Constant(1.0),
                NumericValue::ResultStart,
                NumericValue::StringConstant(0),
                NumericValue::LocalSet(0),
                NumericValue::Constant(2.0),
                NumericValue::ResultReturn(1),
                NumericValue::ResultEnd,
                NumericValue::Drop,
                NumericValue::LocalGet(0),
            ],
            vec![number, None, string, None, number, None, None, None, None],
        );
        assert_eq!(result.callback_result_kind(), Some(CallbackValueKind::String));

        let caught = NumericProgram(
            vec![
                NumericValue::Constant(1.0),
                NumericValue::TryStart,
                NumericValue::StringConstant(0),
                NumericValue::LocalSet(0),
                NumericValue::Constant(2.0),
                NumericValue::CheckError,
                NumericValue::CatchStart,
                NumericValue::TryEnd,
                NumericValue::Drop,
                NumericValue::LocalGet(0),
            ],
            vec![number, None, string, None, number, None, None, None, None, None],
        );
        assert_eq!(caught.callback_result_kind(), Some(CallbackValueKind::String));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn nested_present_conditionals_save_same_slot_state_independently() {
        let values = vec![
            NumericValue::Absent,
            NumericValue::PresentConditionalStart,
            NumericValue::Constant(1.0),
            NumericValue::ConditionalAlternate,
            NumericValue::Null,
            NumericValue::PresentConditionalStart,
            NumericValue::Constant(2.0),
            NumericValue::ConditionalAlternate,
            NumericValue::Absent,
            NumericValue::Constant(3.0),
            NumericValue::PreserveAbsent,
            NumericValue::Drop,
            NumericValue::ShortCircuitEnd,
            NumericValue::PreserveAbsent,
            NumericValue::Drop,
            NumericValue::ShortCircuitEnd,
            NumericValue::Drop,
        ];
        let program = NumericProgram(values.clone(), vec![None; values.len()]);
        let code = program.machine_code().expect("nested conditional should compile");
        assert!(code.windows(11).any(|bytes| {
            bytes[..7] == [0x8a, 0x44, 0x24, 0, 0x88, 0x84, 0x24]
                && bytes[7..11] == 32u32.to_le_bytes()
        }));
        assert!(code.windows(11).any(|bytes| {
            bytes[..7] == [0x8a, 0x44, 0x24, 0, 0x88, 0x84, 0x24]
                && bytes[7..11] == 33u32.to_le_bytes()
        }));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn jit_callback_result_scopes_presence_and_preserves_nullish_state() {
        extern "C" fn returns_null(_: *const f64) -> f64 {
            CALL_ABSENCE.with(|absence| absence.set(2));
            CALL_PRESENT.with(|present| present.set(false));
            0.0
        }
        let previous_present = CALL_PRESENT.with(|present| present.replace(true));
        let previous_absence = CALL_ABSENCE.with(|absence| absence.replace(1));
        let result = call_jit_callback(returns_null, &[], &[]);
        assert_eq!(result.word, 0.0);
        assert!(matches!(result.presence, JitCallbackPresence::Null));
        assert!(CALL_PRESENT.with(Cell::get));
        assert_eq!(CALL_ABSENCE.with(Cell::get), 1);
        CALL_PRESENT.with(|present| present.set(previous_present));
        CALL_ABSENCE.with(|absence| absence.set(previous_absence));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn maybe_dynamic_tags_encode_undefined_null_and_present_values() {
        let previous_error = CALL_ERROR.with(|error| error.replace(std::ptr::null()));
        let missing_string = tag_primitive_with_presence(
            f64::from_bits(1),
            1.0,
            1.0,
        );
        let undefined = tag_primitive_with_presence(0.0, 0.0, 1.0);
        let null = tag_primitive_with_presence(0.0, 0.0, 2.0);
        let zero = tag_primitive_with_presence(0.0, 0.0, 0.0);
        let another_undefined = tag_primitive_with_presence(0.0, 3.0, 1.0);
        let another_null = tag_primitive_with_presence(0.0, 3.0, 2.0);

        assert_eq!(dynamic_tag(missing_string), DYNAMIC_UNDEFINED_TAG as f64);
        assert_eq!(dynamic_compare(undefined, another_undefined, 6), 1.0);
        assert_eq!(dynamic_compare(null, another_null, 6), 1.0);
        assert_eq!(dynamic_compare(undefined, null, 6), 0.0);
        assert_eq!(dynamic_compare(undefined, null, 4), 1.0);
        assert_eq!(dynamic_compare(null, zero, 4), 0.0);
        assert_eq!(dynamic_compare(undefined, zero, 6), 0.0);
        assert!(CALL_ERROR.with(Cell::get).is_null());
        CALL_ERROR.with(|error| error.set(previous_error));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn callback_kind_flow_reaches_loop_local_fixedpoint_and_uses_exit_edges() {
        let program = NumericProgram(
            vec![
                NumericValue::LoopStart,
                NumericValue::Constant(1.0),
                NumericValue::LocalSet(0),
                NumericValue::Constant(1.0),
                NumericValue::LoopWhile,
                NumericValue::Constant(1.0),
                NumericValue::TypeOfNumber,
                NumericValue::LocalSet(0),
                NumericValue::LoopContinuePoint,
                NumericValue::LoopEnd,
                NumericValue::LocalGet(0),
            ],
            vec![
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                None,
                Some(CallbackSourceKind::Constant(CallbackValueKind::Number)),
                None,
                None,
                None,
                None,
                None,
            ],
        );
        assert_eq!(program.callback_result_kind(), Some(CallbackValueKind::Number));
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn dynamic_argument_requires_two_available_registers() {
        for (depth, accepted) in [(6, true), (7, false)] {
            let mut values = vec![NumericValue::Constant(1.0); depth];
            values.push(NumericValue::DynamicArgument(0));
            values.extend(std::iter::repeat(NumericValue::Operation(NumericOp::Add)).take(depth));
            let source_kinds = vec![None; values.len()];
            assert_eq!(NumericProgram(values, source_kinds).machine_code().is_some(), accepted);
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn string_well_formed_operations_handle_lone_surrogates() {
        let check = CString::new("expr:s0,iswellformed:iswellformed").unwrap();
        let repair = CString::new("expr:s0,towellformed:towellformed").unwrap();
        for (units, expected) in [
            (vec![0xd800, b'x' as u16, 0xdc00], "\u{fffd}x\u{fffd}"),
            (vec![0xd800, 0xdc00, 0xd800], "\u{10000}\u{fffd}"),
        ] {
            let input = CString::new(thaw_arena::wtf8_encode_utf16(&units)).unwrap();
            let argument = f64::from_bits(input.as_ptr() as usize as u64);
            let checked = call(&check, &[argument]);
            assert!(checked.error.is_null());
            assert_eq!(checked.value, 0.0);
            let result = call(&repair, &[argument]);
            assert!(result.error.is_null());
            let output = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(unsafe { CStr::from_ptr(output) }.to_bytes(), expected.as_bytes());
            unsafe { libc::free(output.cast()) };
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn reversed_helper_calls_preserve_outer_values() {
        for (operation, args, expected) in [
            ("rpow", [19.0, 3.0, 2.0], 27.0),
            ("rrem", [19.0, 3.0, 10.0], 20.0),
        ] {
            let symbol = CString::new(format!("expr:a0,a1,a2,{operation},+:outer-{operation}")).unwrap();
            let result = call(&symbol, &args);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn nullish_tuple_reads_preserve_null_and_undefined() {
        for kind in ["n", "b", "s"] {
            let symbol = CString::new(format!("expr:a0,tupnull{kind}0:tuple-absence")).unwrap();
            for (tag, expected) in [(1_u64, NULL_STATUS), (2_u64, ABSENT_STATUS)] {
                let data = [1_u64, tag, 0];
                let handle = data.as_ptr().cast::<u8>();
                let argument = f64::from_bits((&handle as *const *const u8) as usize as u64);
                let result = call(&symbol, &[argument]);
                assert_eq!(result.error, expected);
            }
        }
    }

    #[cfg(all(target_arch = "x86_64", target_family = "unix"))]
    #[test]
    fn dynamic_array_search_type_mismatch_is_not_an_exception() {
        let data = [0_u64];
        let handle = data.as_ptr().cast::<u8>();
        let array = f64::from_bits((&handle as *const *const u8) as usize as u64);
        for (array_tag, needle_tag) in [("tagrn", "tagbool"), ("tagrb", "tagnum"), ("tagrs", "tagnum")] {
            for (operation, expected) in [("dynarrayincludes", 0.0), ("dynarrayindexof", -1.0), ("dynarraylastindexof", -1.0)] {
                let symbol = CString::new(format!("expr:a0,{array_tag},a1,{needle_tag},a2,{operation}:mismatch")).unwrap();
                let result = call(&symbol, &[array, 1.0, 0.0]);
                assert!(result.error.is_null());
                assert_eq!(result.value, expected);
            }
        }
        let invalid = CString::new("expr:a0,tagnum,a1,tagbool,a2,dynarrayincludes:invalid-receiver").unwrap();
        assert!(!call(&invalid, &[1.0, 1.0, 0.0]).error.is_null());
        let null_array = CString::new("expr:a0,tagrn,a1,tagbool,a2,dynarrayincludes:null-array").unwrap();
        assert!(!call(&null_array, &[f64::from_bits(0), 1.0, 0.0]).error.is_null());
    }

    fn call(symbol: &CString, args: &[f64]) -> ThawJitResult {
        unsafe extern "C" fn allocate(size: usize, _: usize) -> *mut u8 {
            unsafe { libc::malloc(size).cast() }
        }
        unsafe extern "C" fn format_number_string(_: f64) -> *const c_char {
            c"42".as_ptr()
        }
        unsafe extern "C" fn parse_string(_: *const c_char) -> f64 {
            42.0
        }
        unsafe extern "C" fn random() -> f64 {
            0.25
        }
        unsafe extern "C" fn wall_now() -> f64 {
            1_000.0
        }
        unsafe extern "C" fn monotonic_now() -> f64 {
            10.0
        }
        unsafe extern "C" fn pid() -> f64 {
            123.0
        }
        unsafe extern "C" fn ppid() -> f64 {
            12.0
        }
        unsafe extern "C" fn parse_float(value: *const c_char) -> f64 {
            match unsafe { CStr::from_ptr(value) }.to_bytes() {
                b"  -12.5px" => -12.5,
                _ => 42.0,
            }
        }
        unsafe extern "C" fn parse_int(value: *const c_char, radix: f64) -> f64 {
            match (unsafe { CStr::from_ptr(value) }.to_bytes(), radix) {
                (b"11", 2.0) => 3.0,
                _ => 42.0,
            }
        }
        unsafe extern "C" fn format_method(
            operation: u8,
            value: f64,
            argument: f64,
        ) -> *const c_char {
            let text = match (operation, value, argument) {
                (0, 12.5, 1.0) => "12.5",
                (1, 12.5, 3.0) => "12.5",
                (2, 255.0, 16.0) => "ff",
                (3, 12.5, 1.0) => "1.3e+1",
                (3, 12.5, -1.0) => "1.25e+1",
                _ => return ptr::null(),
            };
            let output = unsafe { libc::malloc(text.len() + 1).cast::<u8>() };
            unsafe {
                ptr::copy_nonoverlapping(text.as_ptr(), output, text.len());
                output.add(text.len()).write(0);
            }
            output.cast()
        }
        unsafe extern "C" fn search_array(
            operation: u8,
            array: *const u8,
            presence: *const u8,
            needle: f64,
            from_index: f64,
        ) -> f64 {
            assert!(!array.is_null());
            assert!(presence.is_null());
            match (operation, needle, from_index) {
                (0, 20.0, 0.0) => 1.0,
                (1, 20.0, 0.0) => 1.0,
                (6, 20.0, f64::INFINITY) => 1.0,
                _ => -1.0,
            }
        }
        unsafe extern "C" fn format_array(
            operation: u8,
            array: *const u8,
            presence: *const u8,
            separator: *const c_char,
        ) -> *const c_char {
            assert_eq!(operation, 0);
            assert!(!array.is_null());
            assert!(presence.is_null());
            match unsafe { CStr::from_ptr(separator) }.to_bytes() {
                b"|" => c"10|20|30".as_ptr(),
                b"," => c"10,20,30".as_ptr(),
                separator => panic!("unexpected separator {separator:?}"),
            }
        }
        unsafe extern "C" fn normalize_string(
            value: *const c_char,
            form: *const c_char,
        ) -> *const c_char {
            assert_eq!(unsafe { CStr::from_ptr(value) }.to_bytes(), "é".as_bytes());
            match unsafe { CStr::from_ptr(form) }.to_bytes() {
                b"NFC" => c"é".as_ptr(),
                _ => ptr::null(),
            }
        }
        unsafe extern "C" fn split_string(
            value: *const c_char,
            separator: *const c_char,
            limit: f64,
        ) -> *mut u8 {
            assert_eq!(unsafe { CStr::from_ptr(value) }.to_bytes(), b"a,b");
            assert_eq!(unsafe { CStr::from_ptr(separator) }.to_bytes(), b",");
            assert_eq!(limit, 1.0);
            let output = unsafe { libc::malloc(16).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(1);
                output.add(8).cast::<*const c_char>().write(c"a".as_ptr());
            }
            output
        }
        unsafe extern "C" fn convert_string(value: *const c_char) -> *mut u8 {
            assert_eq!(unsafe { CStr::from_ptr(value) }.to_bytes(), "😀".as_bytes());
            let output = unsafe { libc::malloc(16).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(1);
                output.add(8).cast::<*const c_char>().write(c"😀".as_ptr());
            }
            output
        }
        unsafe extern "C" fn from_char_code(value: f64) -> *const c_char {
            if value == 65.0 {
                c"A".as_ptr()
            } else {
                ptr::null()
            }
        }
        unsafe extern "C" fn from_code_point(value: f64) -> *const c_char {
            if value == 0x1f600 as f64 {
                c"😀".as_ptr()
            } else {
                ptr::null()
            }
        }
        unsafe extern "C" fn slice_array(
            array: *const u8,
            element_width: usize,
            start: f64,
            end: f64,
        ) -> *mut u8 {
            assert!(!array.is_null());
            assert_eq!(element_width, 8);
            assert_eq!((start, end), (1.0, 2.0));
            let output = unsafe { libc::malloc(16).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(1);
                output.add(8).cast::<f64>().write(20.0);
            }
            output
        }
        unsafe extern "C" fn reverse_array(array: *const u8, element_width: usize) -> *mut u8 {
            assert!(!array.is_null());
            assert_eq!(element_width, 8);
            let output = unsafe { libc::malloc(32).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(3);
                for (index, value) in [30.0_f64, 20.0, 10.0].into_iter().enumerate() {
                    output.add(8 + index * 8).cast::<f64>().write(value);
                }
            }
            output
        }
        unsafe extern "C" fn reverse_array_in_place(
            array: *mut u8,
            element_width: usize,
        ) -> *mut u8 {
            assert!(!array.is_null());
            assert_eq!(element_width, 8);
            array
        }
        unsafe extern "C" fn concatenate_arrays(
            left: *const u8,
            right: *const u8,
            element_width: usize,
        ) -> *mut u8 {
            assert!(!left.is_null() && !right.is_null());
            assert_eq!(element_width, 8);
            let left_len = unsafe { left.cast::<u64>().read() } as usize;
            let right_len = unsafe { right.cast::<u64>().read() } as usize;
            let output = unsafe { libc::malloc(8 + (left_len + right_len) * 8).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write((left_len + right_len) as u64);
                std::ptr::copy_nonoverlapping(left.add(8), output.add(8), left_len * 8);
                std::ptr::copy_nonoverlapping(
                    right.add(8),
                    output.add(8 + left_len * 8),
                    right_len * 8,
                );
            }
            output
        }
        unsafe extern "C" fn append_array_value(
            operation: u8,
            array: *const u8,
            value: f64,
        ) -> *mut u8 {
            assert_eq!(operation, 0);
            let length = unsafe { array.cast::<u64>().read() } as usize;
            let output = unsafe { libc::malloc(8 + (length + 1) * 8).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write((length + 1) as u64);
                std::ptr::copy_nonoverlapping(array.add(8), output.add(8), length * 8);
                output.add(8 + length * 8).cast::<f64>().write(value);
            }
            output
        }
        unsafe extern "C" fn sort_array(operation: u8, array: *const u8) -> *mut u8 {
            assert_eq!(operation, 0);
            assert!(!array.is_null());
            let output = unsafe { libc::malloc(32).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(3);
                for (index, value) in [10.0_f64, 20.0, 30.0].into_iter().enumerate() {
                    output.add(8 + index * 8).cast::<f64>().write(value);
                }
            }
            output
        }
        unsafe extern "C" fn sort_array_in_place(operation: u8, array: *mut u8) -> *mut u8 {
            assert_eq!(operation, 0);
            assert!(!array.is_null());
            array
        }
        unsafe extern "C" fn fill_array_in_place(
            operation: u8,
            array: *mut u8,
            value: f64,
            start: f64,
            end: f64,
        ) -> *mut u8 {
            assert_eq!((operation, value, start, end), (0, 20.0, 1.0, 2.0));
            assert!(!array.is_null());
            array
        }
        unsafe extern "C" fn copy_array_within(
            array: *mut u8,
            element_width: usize,
            target: f64,
            start: f64,
            end: f64,
        ) -> *mut u8 {
            assert_eq!((element_width, target, start, end), (8, 0.0, 1.0, 2.0));
            assert!(!array.is_null());
            array
        }
        unsafe extern "C" fn push_array_value(
            operation: u8,
            array: *mut *mut u8,
            value: f64,
        ) -> f64 {
            assert_eq!((operation, value), (0, 20.0));
            assert!(!array.is_null());
            4.0
        }
        unsafe extern "C" fn remove_array_value(
            operation: u8,
            array: *mut *mut u8,
            out_value: *mut f64,
        ) -> i8 {
            assert_eq!(operation, 0);
            assert!(!array.is_null() && !out_value.is_null());
            unsafe { out_value.write(20.0) };
            1
        }
        unsafe extern "C" fn replace_array(
            operation: u8,
            array: *const u8,
            index: f64,
            value: f64,
        ) -> *mut u8 {
            assert_eq!((operation, index, value), (0, 1.0, 99.0));
            assert!(!array.is_null());
            let output = unsafe { libc::malloc(32).cast::<u8>() };
            unsafe {
                output.cast::<u64>().write(3);
                for (index, value) in [10.0_f64, 99.0, 30.0].into_iter().enumerate() {
                    output.add(8 + index * 8).cast::<f64>().write(value);
                }
            }
            output
        }
        unsafe {
            thaw_jit_call_f64(
                symbol.as_ptr(),
                args.as_ptr(),
                args.len(),
                Some(allocate),
                Some(format_number_string),
                Some(parse_string),
                Some(parse_float),
                Some(parse_int),
                Some(format_method),
                Some(search_array),
                Some(format_array),
                Some(normalize_string),
                Some(split_string),
                Some(slice_array),
                Some(concatenate_arrays),
                Some(append_array_value),
                Some(reverse_array),
                Some(sort_array),
                Some(reverse_array_in_place),
                Some(sort_array_in_place),
                Some(fill_array_in_place),
                Some(copy_array_within),
                Some(push_array_value),
                Some(push_array_value),
                Some(remove_array_value),
                None,
                None,
                Some(replace_array),
                Some(random),
                Some(wall_now),
                Some(monotonic_now),
                Some(pid),
                Some(ppid),
                Some(convert_string),
                Some(from_char_code),
                Some(from_code_point),
                None,
                None,
                None,
                None,
                None,
                None,
            )
        }
    }

    #[test]
    fn carries_tagged_number_and_string_primitives_in_one_jit_slot() {
        let boxed_number = CString::new("expr:a0,tagnum:boxed-number").unwrap();
        let boxed = call(&boxed_number, &[20.0]);
        assert!(boxed.error.is_null());
        let boxed = unsafe { &*(boxed.value.to_bits() as usize as *const DynamicPrimitive) };
        assert_eq!(
            (boxed.tag, f64::from_bits(boxed.payload)),
            (DYNAMIC_NUMBER_TAG, 20.0)
        );

        let number_tag = CString::new("expr:a0,tagnum,tagkind:number-tag").unwrap();
        let result = call(&number_tag, &[20.0]);
        assert!(
            result.error.is_null(),
            "{}",
            unsafe { CStr::from_ptr(result.error) }.to_string_lossy()
        );
        assert_eq!(result.value, DYNAMIC_NUMBER_TAG as f64);

        for (symbol, expected) in [
            ("expr:a0,tagnum,typeofdynamic:number-type", "number"),
            ("expr:t78,tagstr,typeofdynamic:string-type", "string"),
            ("expr:b0,tagbool,typeofdynamic:boolean-type", "boolean"),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[20.0]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                expected
            );
        }

        let number = CString::new("expr:a0,tagnum,untagnum:number").unwrap();
        let result = call(&number, &[20.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 20.0);

        let string = CString::new("expr:t68656c6c6f,tagstr,untagstr:string").unwrap();
        let result = call(&string, &[]);
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char).to_bytes() },
            b"hello"
        );
        let boolean = CString::new("expr:b0,tagbool,notnum,notstr,untagbool:boolean").unwrap();
        let result = call(&boolean, &[1.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let dynamic_argument = CString::new("expr:u0nbs,untagbool:dynamic-argument").unwrap();
        let result = call(&dynamic_argument, &[3.0, f64::from_bits(1)]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);
        for tag in DYNAMIC_NUMBER_ARRAY_TAG..=DYNAMIC_STRING_DICTIONARY_TAG {
            let result = call(
                &CString::new("expr:u0NBSD,typeofdynamic:aggregate-type").unwrap(),
                &[tag as f64, 0.0],
            );
            assert!(result.error.is_null());
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                "object"
            );
        }
        for (symbol, expected) in [
            (
                "expr:arrayempty,tagrn,tagkind:number-array-tag",
                DYNAMIC_NUMBER_ARRAY_TAG,
            ),
            (
                "expr:arrayempty,tagrb,tagkind:boolean-array-tag",
                DYNAMIC_BOOLEAN_ARRAY_TAG,
            ),
            (
                "expr:arrayempty,tagrs,tagkind:string-array-tag",
                DYNAMIC_STRING_ARRAY_TAG,
            ),
            (
                "expr:dnempty,tagdn,tagkind:number-dictionary-tag",
                DYNAMIC_NUMBER_DICTIONARY_TAG,
            ),
            (
                "expr:dbempty,tagdb,tagkind:boolean-dictionary-tag",
                DYNAMIC_BOOLEAN_DICTIONARY_TAG,
            ),
            (
                "expr:dsempty,tagds,tagkind:string-dictionary-tag",
                DYNAMIC_STRING_DICTIONARY_TAG,
            ),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(result.value, expected as f64, "{symbol}");
        }

        for (symbol, expected) in [
            ("expr:c0000000000000000,tagnum,dynbool:false-number", 0.0),
            ("expr:c7ff8000000000000,tagnum,dynbool:nan", 0.0),
            ("expr:c3ff0000000000000,tagnum,dynbool:true-number", 1.0),
            ("expr:t,tagstr,dynbool:false-string", 0.0),
            ("expr:t78,tagstr,dynbool:true-string", 1.0),
            ("expr:c0000000000000000,tagbool,dynbool:false-boolean", 0.0),
            ("expr:c3ff0000000000000,tagbool,dynbool:true-boolean", 1.0),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(result.value, expected, "{symbol}");
        }

        let result = call(
            &CString::new("expr:t3432,tagstr,dynnum:dynamic-number").unwrap(),
            &[],
        );
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);
        let result = call(
            &CString::new("expr:c4000000000000000,tagnum,dynstr:dynamic-string").unwrap(),
            &[],
        );
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "42"
        );

        let result = call(
            &CString::new("expr:t61,tagstr,t62,tagstr,dynadd,untagstr:dynamic-string-add").unwrap(),
            &[],
        );
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "ab"
        );
        for (symbol, expected) in [
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dyneq:loose-equal",
                1.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dynlte:less-equal",
                1.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dyngt:greater",
                0.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dyngte:greater-equal",
                1.0,
            ),
            (
                "expr:c4045000000000000,tagnum,t3432,tagstr,dynne:not-equal",
                0.0,
            ),
            (
                "expr:c4000000000000000,tagnum,t32,tagstr,dynseq:strict-equal",
                0.0,
            ),
            (
                "expr:c4000000000000000,tagnum,t32,tagstr,dynsne:strict-not-equal",
                1.0,
            ),
            ("expr:t61,tagstr,t62,tagstr,dynlt:string-less", 1.0),
            (
                "expr:c3ff0000000000000,tagbool,c3ff0000000000000,tagnum,dyneq:boolean-number",
                1.0,
            ),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert!(result.error.is_null(), "{symbol}");
            assert_eq!(result.value, expected, "{symbol}");
        }

        let mismatch = CString::new("expr:t68656c6c6f,tagstr,untagnum:mismatch").unwrap();
        assert!(!call(&mismatch, &[]).error.is_null());

        let malformed = CString::new("expr:a0,untagnum:malformed").unwrap();
        assert!(!call(&malformed, &[42.0]).error.is_null());
    }

    #[test]
    fn short_circuit_skips_unselected_native_calls() {
        let one = format!("c{:016x}", 1.0f64.to_bits());
        let zero = format!("c{:016x}", 0.0f64.to_bits());
        let invalid_digits = format!("c{:016x}", 101.0f64.to_bits());
        let selected = CString::new(format!(
            "expr:{one},dup,asbool,||,{one},{invalid_digits},tofixed,end:short-or"
        ))
        .unwrap();
        let result = call(&selected, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let rejected = CString::new(format!(
            "expr:{zero},dup,asbool,||,{one},{invalid_digits},tofixed,end:run-or"
        ))
        .unwrap();
        assert!(!call(&rejected, &[]).error.is_null());

        let consequent = CString::new(format!(
            "expr:{one},if,{one},else,{one},{invalid_digits},tofixed,end:conditional-true"
        ))
        .unwrap();
        let result = call(&consequent, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let alternate = CString::new(format!(
            "expr:{zero},if,{one},{invalid_digits},tofixed,else,{one},end:conditional-false"
        ))
        .unwrap();
        let result = call(&alternate, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let two = format!("c{:016x}", 2.0f64.to_bits());
        let three = format!("c{:016x}", 3.0f64.to_bits());
        let four = format!("c{:016x}", 4.0f64.to_bits());
        for (condition, expected) in [(&one, 3.0), (&zero, 7.0)] {
            let multiple = CString::new(format!(
                "expr:{condition},if,{one},{two},else,{three},{four},end,+:conditional-multiple"
            ))
            .unwrap();
            let result = call(&multiple, &[]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let five = format!("c{:016x}", 5.0f64.to_bits());
        let ten = format!("c{:016x}", 10.0f64.to_bits());
        let twenty = format!("c{:016x}", 20.0f64.to_bits());
        let thirty = format!("c{:016x}", 30.0f64.to_bits());
        let forty = format!("c{:016x}", 40.0f64.to_bits());
        let try_values = CString::new(format!(
            "expr:trystart,a0,asbool,if,{five},throw,{zero},else,{zero},end,drop,{zero},{ten},{twenty},catch,{thirty},{forty},tryend,+,nip:try-multiple"
        ))
        .unwrap();
        for (throws, expected) in [(0.0, 30.0), (1.0, 70.0)] {
            let result = call(&try_values, &[throws]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let early_values = CString::new(format!(
            "expr:resultstart,a0,asbool,guard,{ten},{twenty},resultreturn,guardend,{thirty},{forty},resultend,+:early-multiple"
        ))
        .unwrap();
        for (early, expected) in [(0.0, 70.0), (1.0, 30.0)] {
            let result = call(&early_values, &[early]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let nested_loop_values = CString::new(format!(
            "expr:{zero},resultstart,{zero},loop,ln1,{one},<,while,a0,asbool,guard,{ten},{twenty},resultreturn2,guardend,looptail,ln1,{one},+,setl1,loopend,drop,{thirty},{forty},resultend,+,nip:nested-loop-early-multiple"
        ))
        .unwrap();
        for (early, expected) in [(0.0, 70.0), (1.0, 30.0)] {
            let result = call(&nested_loop_values, &[early]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let outer_break = CString::new(format!(
            "expr:{zero},loop,{one},asbool,while,loop,{one},asbool,while,break1,looptail,loopend,looptail,loopend,ln0,nip:outer-break"
        ))
        .unwrap();
        let result = call(&outer_break, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 0.0);

        let outer_continue = CString::new(format!(
            "expr:{zero},loop,ln0,{two},<,while,loop,{one},asbool,while,ln0,{one},+,setl0,continue1,looptail,loopend,looptail,loopend,ln0,nip:outer-continue"
        ))
        .unwrap();
        let result = call(&outer_continue, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 2.0);

        let nested_switch_values = CString::new(format!(
            "expr:{zero},resultstart,a0,switch,case,dup,{five},==,casebody,{ten},{twenty},resultreturn2,default,switchbreak,switchend,{thirty},{forty},resultend,+,nip:nested-switch-early-multiple"
        ))
        .unwrap();
        for (value, expected) in [(1.0, 70.0), (5.0, 30.0)] {
            let result = call(&nested_switch_values, &[value]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        for (name, clauses, cases) in [
            ("default-first", format!("default,ln0,{ten},+,setl0,case,dup,{one},==,casebody,ln0,{twenty},+,setl0,switchbreak"), vec![(1.0, 20.0), (2.0, 30.0)]),
            ("default-first-break", format!("default,ln0,{ten},+,setl0,switchbreak,case,dup,{one},==,casebody,ln0,{twenty},+,setl0,switchbreak"), vec![(1.0, 20.0), (2.0, 10.0)]),
            ("default-middle", format!("case,dup,{one},==,casebody,ln0,{ten},+,setl0,default,ln0,{twenty},+,setl0,case,dup,{two},==,casebody,ln0,{forty},+,setl0,switchbreak"), vec![(1.0, 70.0), (2.0, 40.0), (3.0, 60.0)]),
            ("default-last", format!("case,dup,{one},==,casebody,ln0,{ten},+,setl0,default,ln0,{twenty},+,setl0"), vec![(1.0, 30.0), (2.0, 20.0)]),
        ] {
            let symbol = CString::new(format!("expr:{zero},a0,switch,{clauses},switchend,ln0,nip:{name}")).unwrap();
            for (value, expected) in cases {
                let result = call(&symbol, &[value]);
                assert!(result.error.is_null());
                assert_eq!(result.value, expected, "{name}: {value}");
            }
        }

        let optional_present = CString::new(format!(
            "expr:{one},if,{one},else,absentn,end:optional-present"
        ))
        .unwrap();
        assert!(call(&optional_present, &[]).error.is_null());
        let optional_absent = CString::new(format!(
            "expr:{zero},if,{one},else,absentn,end:optional-absent"
        ))
        .unwrap();
        assert_eq!(call(&optional_absent, &[]).error, ABSENT_STATUS);

        let present_coalesce = CString::new(format!(
            "expr:{one},ifpresent,else,{one},{invalid_digits},tofixed,end:present-coalesce"
        ))
        .unwrap();
        let result = call(&present_coalesce, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);
        let absent_coalesce = CString::new(format!(
            "expr:absentn,ifpresent,else,{one},end:absent-coalesce"
        ))
        .unwrap();
        let result = call(&absent_coalesce, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);
    }

    #[test]
    fn recursively_calls_the_compiled_expression() {
        let factorial = CString::new(
            "expr:a0,c3ff0000000000000,<=,if,c3ff0000000000000,else,a0,a0,c3ff0000000000000,-,recurn1,*,end:factorial",
        )
        .unwrap();
        let result = call(&factorial, &[6.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 720.0);

        let gcd =
            CString::new("expr:a1,c0000000000000000,==,if,a0,else,a1,a0,a1,%,recurn2,end:gcd")
                .unwrap();
        let result = call(&gcd, &[1071.0, 462.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 21.0);
    }

    #[test]
    fn specializes_numeric_operations_and_reuses_code() {
        for (symbol, expected) in [
            ("add:test", 42.0),
            ("sub:test", 0.0),
            ("mul:test", 441.0),
            ("div:test", 1.0),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[21.0, 21.0]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
            let repeated = call(&symbol, &[20.0, 22.0]);
            assert!(repeated.error.is_null());
        }

        let symbol = CString::new("expr:x,y,+,c4000000000000000,*:compound").unwrap();
        let result = call(&symbol, &[19.0, 2.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);

        let symbol = CString::new("expr:a0,a1,+,a2,+:three_args").unwrap();
        let result = call(&symbol, &[10.0, 11.0, 21.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);
        assert!(!call(&symbol, &[10.0, 11.0]).error.is_null());

        let symbol =
            CString::new("expr:x,y,<,c4045000000000000,cc000000000000000,?:conditional").unwrap();
        let comparison = CString::new("expr:x,y,<:comparison").unwrap();
        assert_eq!(call(&comparison, &[2.0, 3.0]).value, 1.0);
        assert_eq!(call(&symbol, &[2.0, 3.0]).value, 42.0);
        assert_eq!(call(&symbol, &[3.0, 2.0]).value, -2.0);

        for (operator, expected) in [("<", 0.0), ("==", 0.0), ("!=", 1.0)] {
            let symbol = CString::new(format!("expr:x,y,{operator}:nan")).unwrap();
            let result = call(&symbol, &[f64::NAN, 1.0]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }

        let symbol =
            CString::new("expr:x,c4045000000000000,cc000000000000000,?:truthiness").unwrap();
        assert_eq!(call(&symbol, &[-0.0]).value, -2.0);
        assert_eq!(call(&symbol, &[f64::NAN]).value, -2.0);

        let symbol = CString::new("expr:a0,neg:negate").unwrap();
        assert_eq!(call(&symbol, &[42.0]).value, -42.0);
        assert_eq!(call(&symbol, &[-0.0]).value.to_bits(), 0.0f64.to_bits());

        let symbol = CString::new("expr:c4045000000000000:constant").unwrap();
        assert_eq!(call(&symbol, &[]).value, 42.0);

        let symbol = CString::new("expr:a0,a1,%:remainder").unwrap();
        assert_eq!(call(&symbol, &[5.5, 2.0]).value, 1.5);
        assert_eq!(call(&symbol, &[-5.5, 2.0]).value, -1.5);
        assert!(call(&symbol, &[f64::INFINITY, 2.0]).value.is_nan());

        let symbol = CString::new("expr:a0,abs:absolute").unwrap();
        assert_eq!(call(&symbol, &[-42.0]).value, 42.0);
        assert_eq!(call(&symbol, &[-0.0]).value.to_bits(), 0.0f64.to_bits());
        assert!(call(&symbol, &[f64::NAN]).value.is_nan());

        let minimum = CString::new("expr:a0,a1,min:minimum").unwrap();
        let maximum = CString::new("expr:a0,a1,max:maximum").unwrap();
        assert_eq!(call(&minimum, &[42.0, 43.0]).value, 42.0);
        assert_eq!(call(&maximum, &[41.0, 42.0]).value, 42.0);
        assert!(call(&minimum, &[f64::NAN, 42.0]).value.is_nan());
        assert!(call(&maximum, &[42.0, f64::NAN]).value.is_nan());
        assert_eq!(
            call(&minimum, &[0.0, -0.0]).value.to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(
            call(&maximum, &[-0.0, 0.0]).value.to_bits(),
            0.0f64.to_bits()
        );

        for (operation, input, expected) in [
            ("floor", -1.2, -2.0),
            ("ceil", -1.2, -1.0),
            ("trunc", -1.2, -1.0),
            ("round", -1.5, -1.0),
            ("sqrt", 9.0, 3.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            assert_eq!(call(&symbol, &[input]).value, expected);
        }
        let round = CString::new("expr:a0,round:round_zero").unwrap();
        assert_eq!(call(&round, &[-0.5]).value.to_bits(), (-0.0f64).to_bits());
        let square_root = CString::new("expr:a0,sqrt:sqrt_nan").unwrap();
        assert!(call(&square_root, &[-1.0]).value.is_nan());

        for (operation, input, expected) in [
            ("acos", 1.0, 0.0),
            ("acosh", 1.0, 0.0),
            ("asin", 0.0, 0.0),
            ("asinh", 0.0, 0.0),
            ("atan", 0.0, 0.0),
            ("atanh", 0.0, 0.0),
            ("cbrt", 8.0, 2.0),
            ("cos", 0.0, 1.0),
            ("cosh", 0.0, 1.0),
            ("exp", 0.0, 1.0),
            ("expm1", 0.0, 0.0),
            ("log", 1.0, 0.0),
            ("log1p", 0.0, 0.0),
            ("log2", 8.0, 3.0),
            ("log10", 100.0, 2.0),
            ("sign", -8.0, -1.0),
            ("sin", 0.0, 0.0),
            ("sinh", 0.0, 0.0),
            ("tan", 0.0, 0.0),
            ("tanh", 0.0, 0.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            let result = call(&symbol, &[input]);
            assert!(result.error.is_null());
            assert!((result.value - expected).abs() < 1e-12);
        }
        let sign = CString::new("expr:a0,sign:sign_special").unwrap();
        assert_eq!(call(&sign, &[-0.0]).value.to_bits(), (-0.0f64).to_bits());
        assert!(call(&sign, &[f64::NAN]).value.is_nan());
        let atan2 = CString::new("expr:a0,a1,atan2:atan2").unwrap();
        assert!((call(&atan2, &[1.0, 1.0]).value - std::f64::consts::FRAC_PI_4).abs() < 1e-12);
        let hypot = CString::new("expr:a0,a1,hypot:hypot").unwrap();
        assert_eq!(call(&hypot, &[3.0, 4.0]).value, 5.0);
        let nested_unary = CString::new("expr:a0,a1,sin,+:nested_unary").unwrap();
        assert!((call(&nested_unary, &[2.0, 1.0]).value - (2.0 + 1.0f64.sin())).abs() < 1e-12);
        let nested_binary = CString::new("expr:a0,a1,a2,hypot,+:nested_binary").unwrap();
        assert_eq!(call(&nested_binary, &[7.0, 3.0, 4.0]).value, 12.0);
        let clz32 = CString::new("expr:a0,clz32:clz32").unwrap();
        assert_eq!(call(&clz32, &[0.0]).value, 32.0);
        assert_eq!(call(&clz32, &[1.0]).value, 31.0);
        assert_eq!(call(&clz32, &[-1.0]).value, 0.0);
        let fround = CString::new("expr:a0,fround:fround").unwrap();
        assert_eq!(call(&fround, &[1.337]).value, 1.337f64 as f32 as f64);
        assert_eq!(call(&fround, &[-0.0]).value.to_bits(), (-0.0f64).to_bits());
        let imul = CString::new("expr:a0,a1,imul:imul").unwrap();
        assert_eq!(call(&imul, &[4_294_967_295.0, 5.0]).value, -5.0);
        let text = CString::new("a😀").unwrap();
        let text_argument = f64::from_bits(text.as_ptr() as usize as u64);
        let string_length = CString::new("expr:s0,strlen:string_length").unwrap();
        assert_eq!(call(&string_length, &[text_argument]).value, 3.0);
        for (operation, index, expected) in [
            ("charcodeat", 0.0, 97.0),
            ("charcodeat", 1.0, 55_357.0),
            ("charcodeat", 2.0, 56_832.0),
            ("charcodeat", -1.0, f64::NAN),
        ] {
            let character = CString::new(format!("expr:s0,a1,{operation}:{operation}")).unwrap();
            let actual = call(&character, &[text_argument, index]).value;
            if expected.is_nan() {
                assert!(actual.is_nan());
            } else {
                assert_eq!(actual, expected);
            }
        }
        for (index, expected) in [(0.0, "a"), (1.0, "�"), (-1.0, "")] {
            let character = CString::new("expr:s0,a1,charat:charat").unwrap();
            let result = call(&character, &[text_argument, index]);
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let at = CString::new("expr:s0,a1,at:at").unwrap();
        let result = call(&at, &[text_argument, -1.0]);
        assert!(result.error.is_null());
        let value = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(unsafe { CStr::from_ptr(value) }.to_str().unwrap(), "�");
        unsafe { libc::free(value.cast()) };
        assert_eq!(call(&at, &[text_argument, 3.0]).error, ABSENT_STATUS);
        let code_point = CString::new("expr:s0,a1,codepointat:codepointat").unwrap();
        let result = call(&code_point, &[text_argument, 1.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 128_512.0);
        assert_eq!(
            call(&code_point, &[text_argument, 3.0]).error,
            ABSENT_STATUS
        );
        let supplementary = CString::new("𐀀").unwrap();
        let bmp = CString::new("\u{e000}").unwrap();
        let string_less =
            CString::new("expr:s0,s1,strcmp,c0000000000000000,<:string_utf16_comparison").unwrap();
        assert_eq!(
            call(
                &string_less,
                &[
                    f64::from_bits(supplementary.as_ptr() as usize as u64),
                    f64::from_bits(bmp.as_ptr() as usize as u64),
                ],
            )
            .value,
            1.0
        );
        let value = CString::new("prefix").unwrap();
        for (operation, search, expected) in [
            ("startswith", "pre", 1.0),
            ("endswith", "fix", 1.0),
            ("includes", "ref", 1.0),
            ("includes", "xyz", 0.0),
        ] {
            let search = CString::new(search).unwrap();
            let predicate = CString::new(format!("expr:s0,s1,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &predicate,
                    &[
                        f64::from_bits(value.as_ptr() as usize as u64),
                        f64::from_bits(search.as_ptr() as usize as u64),
                    ],
                )
                .value,
                expected
            );
        }
        let positioned = CString::new("😀abc😀").unwrap();
        for (operation, search, position, expected) in [
            ("startswith2", "abc", 2.0, 1.0),
            ("endswith2", "😀", 7.0, 1.0),
            ("includes2", "😀", 1.0, 1.0),
            ("indexof2", "😀", 1.0, 5.0),
            ("lastindexof2", "😀", 4.0, 0.0),
            ("lastindexof2", "😀", f64::NAN, 5.0),
            ("lastindexof2", "", f64::NAN, 7.0),
            ("lastindexof2", "😀", f64::NEG_INFINITY, 0.0),
            ("indexof2", "😀", f64::NAN, 0.0),
            ("startswith2", "😀", f64::NAN, 1.0),
            ("endswith2", "😀", f64::NAN, 0.0),
            ("indexof2", "", f64::INFINITY, 7.0),
        ] {
            let search = CString::new(search).unwrap();
            let search_at = CString::new(format!("expr:s0,s1,a2,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &search_at,
                    &[
                        f64::from_bits(positioned.as_ptr() as usize as u64),
                        f64::from_bits(search.as_ptr() as usize as u64),
                        position,
                    ],
                )
                .value,
                expected
            );
        }
        let indexed = CString::new("😀a😀").unwrap();
        let emoji = CString::new("😀").unwrap();
        for (operation, expected) in [("indexof", 0.0), ("lastindexof", 3.0)] {
            let index = CString::new(format!("expr:s0,s1,{operation}:{operation}")).unwrap();
            assert_eq!(
                call(
                    &index,
                    &[
                        f64::from_bits(indexed.as_ptr() as usize as u64),
                        f64::from_bits(emoji.as_ptr() as usize as u64),
                    ],
                )
                .value,
                expected
            );
        }
        for (operation, expected) in [("tolowercase", "straße"), ("touppercase", "STRASSE")] {
            let input = CString::new("Straße").unwrap();
            let convert = CString::new(format!("expr:s0,{operation}:{operation}")).unwrap();
            let result = call(&convert, &[f64::from_bits(input.as_ptr() as usize as u64)]);
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let well_formed = CString::new("expr:s0,iswellformed:iswellformed").unwrap();
        assert_eq!(call(&well_formed, &[text_argument]).value, 1.0);
        let preserve = CString::new("expr:s0,towellformed:towellformed").unwrap();
        assert_eq!(call(&preserve, &[text_argument]).value, text_argument);
        let whitespace = CString::new("\u{feff}  value\u{3000}").unwrap();
        for (operation, expected) in [
            ("trim", "value"),
            ("trimstart", "value\u{3000}"),
            ("trimend", "\u{feff}  value"),
        ] {
            let trim = CString::new(format!("expr:s0,{operation}:{operation}")).unwrap();
            let result = call(
                &trim,
                &[f64::from_bits(whitespace.as_ptr() as usize as u64)],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let repeated = CString::new("ab").unwrap();
        let repeat = CString::new("expr:s0,a1,repeat:repeat").unwrap();
        let result = call(
            &repeat,
            &[f64::from_bits(repeated.as_ptr() as usize as u64), 2.9],
        );
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(unsafe { CStr::from_ptr(result) }.to_str().unwrap(), "abab");
        unsafe { libc::free(result.cast()) };
        assert!(!call(
            &repeat,
            &[f64::from_bits(repeated.as_ptr() as usize as u64), -1.0],
        )
        .error
        .is_null());
        for count in [-0.5, -0.0, 0.0, f64::NAN] {
            let output = call(&repeat, &[f64::from_bits(repeated.as_ptr() as usize as u64), count]);
            assert!(output.error.is_null());
            let output = output.value.to_bits() as usize as *mut c_char;
            assert_eq!(unsafe { CStr::from_ptr(output) }.to_bytes(), b"");
            unsafe { libc::free(output.cast()) };
        }
        for count in [-1.5, f64::NEG_INFINITY, f64::INFINITY] {
            assert!(!call(&repeat, &[f64::from_bits(repeated.as_ptr() as usize as u64), count]).error.is_null());
        }
        let lone = CString::new(vec![0xed, 0xa0, 0x80]).unwrap();
        let output = call(&repeat, &[f64::from_bits(lone.as_ptr() as usize as u64), 2.0]);
        assert!(output.error.is_null());
        let output = output.value.to_bits() as usize as *mut c_char;
        assert_eq!(unsafe { CStr::from_ptr(output) }.to_bytes(), &[0xed, 0xa0, 0x80, 0xed, 0xa0, 0x80]);
        unsafe { libc::free(output.cast()) };
        let raw_case = CString::new(vec![0xed, 0xa0, 0x80, b'A', 0xc3, 0x9f, 0xed, 0xb0, 0x80]).unwrap();
        for (operation, expected) in [
            ("tolowercase", &[0xed, 0xa0, 0x80, b'a', 0xc3, 0x9f, 0xed, 0xb0, 0x80][..]),
            ("touppercase", &[0xed, 0xa0, 0x80, b'A', b'S', b'S', 0xed, 0xb0, 0x80][..]),
        ] {
            let symbol = CString::new(format!("expr:s0,{operation}:{operation}")).unwrap();
            let result = call(&symbol, &[f64::from_bits(raw_case.as_ptr() as usize as u64)]);
            assert!(result.error.is_null());
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(unsafe { CStr::from_ptr(result) }.to_bytes(), expected);
            unsafe { libc::free(result.cast()) };
        }
        let sliced = CString::new("😀abcd").unwrap();
        for (operation, start, expected) in [
            ("slice", -2.0, "cd"),
            ("slice", 2.0, "abcd"),
            ("substring", -2.0, "😀abcd"),
            ("substring", 4.9, "cd"),
        ] {
            let suffix = CString::new(format!("expr:s0,a1,{operation}:{operation}")).unwrap();
            let result = call(
                &suffix,
                &[f64::from_bits(sliced.as_ptr() as usize as u64), start],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        for (operation, start, end, expected) in [
            ("slice2", -4.0, -1.0, "abc"),
            ("slice2", 5.0, 2.0, ""),
            ("substring2", 5.0, 2.0, "abc"),
            ("substring2", f64::NAN, 2.9, "😀"),
        ] {
            let range = CString::new(format!("expr:s0,a1,a2,{operation}:{operation}")).unwrap();
            let result = call(
                &range,
                &[f64::from_bits(sliced.as_ptr() as usize as u64), start, end],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        // UTF-16 ranges may split a surrogate pair and must preserve lone units.
        for (operation, start, end, expected) in [
            ("slice2", 0.0, 1.0, &[0xed, 0xa0, 0xbd][..]),
            ("substring2", 1.0, 2.0, &[0xed, 0xb8, 0x80][..]),
        ] {
            let range = CString::new(format!("expr:s0,a1,a2,{operation}:{operation}")).unwrap();
            let result = call(&range, &[f64::from_bits(sliced.as_ptr() as usize as u64), start, end]);
            assert!(result.error.is_null());
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(unsafe { CStr::from_ptr(result) }.to_bytes(), expected);
            unsafe { libc::free(result.cast()) };
        }
        let padded = CString::new("😀").unwrap();
        let padding = CString::new("ab").unwrap();
        for (operation, expected) in [("padstart", "a😀"), ("padend", "😀a")] {
            let pad = CString::new(format!("expr:s0,a1,s2,{operation}:{operation}")).unwrap();
            let result = call(
                &pad,
                &[
                    f64::from_bits(padded.as_ptr() as usize as u64),
                    3.0,
                    f64::from_bits(padding.as_ptr() as usize as u64),
                ],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let replace_value = CString::new("aba").unwrap();
        for (operation, search, replacement, expected) in [
            ("replace", "a", "x", "xba"),
            ("replaceall", "a", "x", "xbx"),
            ("replaceall", "", "-", "-a-b-a-"),
            ("replace", "b", "$$:$&:$`:$'", "a$:b:a:aa"),
            ("replaceall", "a", "$&$&", "aabaa"),
        ] {
            let search = CString::new(search).unwrap();
            let replacement = CString::new(replacement).unwrap();
            let replace = CString::new(format!("expr:s0,s1,s2,{operation}:{operation}")).unwrap();
            let result = call(
                &replace,
                &[
                    f64::from_bits(replace_value.as_ptr() as usize as u64),
                    f64::from_bits(search.as_ptr() as usize as u64),
                    f64::from_bits(replacement.as_ptr() as usize as u64),
                ],
            );
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let combined = CString::new("expr:s0,t707265,startswith,s0,t666978,endswith,s0,t707265,startswith,?,s0,t726566,includes,s0,t707265,startswith,s0,t666978,endswith,s0,t707265,startswith,?,?:combined_string_predicates").unwrap();
        assert_eq!(
            call(&combined, &[f64::from_bits(value.as_ptr() as usize as u64)]).value,
            1.0
        );
        let name = CString::new("世界").unwrap();
        let concatenate = CString::new("expr:t686920,s0,concat:concatenate").unwrap();
        let result = call(
            &concatenate,
            &[f64::from_bits(name.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(
            unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
            "hi 世界"
        );
        unsafe { libc::free(result.cast()) };
        let constructors = CString::new(
            "expr:c4050400000000000,fromcharcode,c40ff600000000000,fromcodepoint,concat:constructors",
        )
        .unwrap();
        let result = call(&constructors, &[]);
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "A😀"
        );
        let invalid_code_point =
            CString::new("expr:c4131000000000000,fromcodepoint:invalid_code_point").unwrap();
        assert!(!call(&invalid_code_point, &[]).error.is_null());
        let argument = [f64::from_bits(name.as_ptr() as usize as u64)];
        let missing_allocator = unsafe {
            thaw_jit_call_f64(
                concatenate.as_ptr(),
                argument.as_ptr(),
                1,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        };
        assert!(!missing_allocator.error.is_null());

        let decomposed = CString::new("é").unwrap();
        let normalize = CString::new("expr:s0,t4e4643,normalize:normalize").unwrap();
        let result = call(
            &normalize,
            &[f64::from_bits(decomposed.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }.to_bytes(),
            "é".as_bytes()
        );
        let invalid = CString::new("expr:s0,t626f677573,normalize:normalize_invalid").unwrap();
        assert!(!call(
            &invalid,
            &[f64::from_bits(decomposed.as_ptr() as usize as u64)]
        )
        .error
        .is_null());
        let split_value = CString::new("a,b").unwrap();
        let split = CString::new("expr:s0,t2c,c3ff0000000000000,split:split").unwrap();
        let result = call(
            &split,
            &[f64::from_bits(split_value.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(8).cast::<*const c_char>().read()) }.to_bytes(),
            b"a"
        );
        unsafe { libc::free(output.cast()) };
        let emoji = CString::new("😀").unwrap();
        let characters = CString::new("expr:s0,strarray:characters").unwrap();
        let result = call(
            &characters,
            &[f64::from_bits(emoji.as_ptr() as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(8).cast::<*const c_char>().read()) }.to_bytes(),
            "😀".as_bytes()
        );
        unsafe { libc::free(output.cast()) };
        let values = [
            3_u64,
            10.0f64.to_bits(),
            20.0f64.to_bits(),
            30.0f64.to_bits(),
        ];
        let data = values.as_ptr().cast::<u8>();
        let handle = &data as *const *const u8;
        for (operation, expected) in [("rnmin", 10.0), ("rnmax", 30.0)] {
            let symbol = CString::new(format!("expr:rn0,{operation}:array-extreme")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
        }
        let hypot = CString::new("expr:rn0,rnhypot:array-hypot").unwrap();
        assert_eq!(
            call(&hypot, &[f64::from_bits(handle as usize as u64)]).value,
            10.0f64.hypot(20.0).hypot(30.0)
        );
        for (operation, expected, first_expected) in [
            ("add", 68.0, 60.0),
            ("sub", -52.0, -40.0),
            ("mul", 48_000.0, 6_000.0),
            ("div", 8.0 / 10.0 / 20.0 / 30.0, 10.0 / 20.0 / 30.0),
            ("rem", 8.0, 10.0),
            ("pow", f64::INFINITY, f64::INFINITY),
            ("min", 8.0, 10.0),
            ("max", 30.0, 30.0),
        ] {
            let reduce = CString::new(format!(
                "expr:rn0,c4020000000000000,rnreduce{operation}:array-reduce"
            ))
            .unwrap();
            assert_eq!(
                call(&reduce, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
            let reduce = CString::new(format!(
                "expr:rn0,c0000000000000000,rnreduce{operation}0:array-reduce-first"
            ))
            .unwrap();
            assert_eq!(
                call(&reduce, &[f64::from_bits(handle as usize as u64)]).value,
                first_expected
            );
            let (right_expected, last_expected) = match operation {
                "add" => (68.0, 60.0),
                "sub" => (-52.0, 0.0),
                "mul" => (48_000.0, 6_000.0),
                "div" => (8.0 / 30.0 / 20.0 / 10.0, 30.0 / 20.0 / 10.0),
                "rem" => (8.0, 0.0),
                "pow" => (f64::INFINITY, power(power(30.0, 20.0), 10.0)),
                "min" => (8.0, 10.0),
                "max" => (30.0, 30.0),
                _ => unreachable!(),
            };
            for (suffix, expected) in [("", right_expected), ("0", last_expected)] {
                let initial = if suffix.is_empty() {
                    "c4020000000000000"
                } else {
                    "c0000000000000000"
                };
                let reduce = CString::new(format!(
                    "expr:rn0,{initial},rnreduceright{operation}{suffix}:array-reduce-right"
                ))
                .unwrap();
                assert_eq!(
                    call(&reduce, &[f64::from_bits(handle as usize as u64)]).value,
                    expected
                );
            }
        }
        for (operation, operand, some, every) in [
            ("lt", 25.0, true, false),
            ("lte", 30.0, true, true),
            ("gt", 25.0, true, false),
            ("gte", 10.0, true, true),
            ("eq", 20.0, true, false),
            ("ne", 20.0, true, false),
            ("eq", f64::NAN, false, false),
            ("ne", f64::NAN, true, true),
        ] {
            for (quantifier, expected) in [("some", some), ("every", every)] {
                let symbol = CString::new(format!(
                    "expr:rn0,c{:016x},rn{quantifier}{operation}:array-{quantifier}",
                    operand.to_bits()
                ))
                .unwrap();
                assert_eq!(
                    call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                    f64::from(expected)
                );
            }
        }
        for (method, expected) in [
            ("find", 20.0),
            ("findindex", 1.0),
            ("findlast", 30.0),
            ("findlastindex", 2.0),
        ] {
            let symbol = CString::new(format!(
                "expr:rn0,c402e000000000000,rn{method}gt:array-{method}"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
        }
        for method in ["find", "findlast"] {
            let symbol = CString::new(format!(
                "expr:rn0,c4058c00000000000,rn{method}eq:array-{method}-missing"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).error,
                ABSENT_STATUS
            );
        }
        for method in ["findindex", "findlastindex"] {
            let symbol = CString::new(format!(
                "expr:rn0,c4058c00000000000,rn{method}eq:array-{method}-missing"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                -1.0
            );
        }
        let filter = CString::new("expr:rn0,c402e000000000000,rnfiltergt:array-filter").unwrap();
        let result = call(&filter, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 2);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 20.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 30.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let truthy_values = [
            4_u64,
            0.0f64.to_bits(),
            f64::NAN.to_bits(),
            (-2.0f64).to_bits(),
            3.0f64.to_bits(),
        ];
        let truthy_data = truthy_values.as_ptr().cast::<u8>();
        let truthy_handle = &truthy_data as *const *const u8;
        for (method, expected) in [
            ("some", 1.0),
            ("every", 0.0),
            ("find", -2.0),
            ("findindex", 2.0),
            ("findlast", 3.0),
            ("findlastindex", 3.0),
        ] {
            let symbol =
                CString::new(format!("expr:rn0,rn{method}truthy:array-{method}-truthy")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(truthy_handle as usize as u64)]).value,
                expected,
                "{method}"
            );
        }
        let filter_truthy = CString::new("expr:rn0,rnfiltertruthy:array-filter-truthy").unwrap();
        let result = call(
            &filter_truthy,
            &[f64::from_bits(truthy_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 2);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, -2.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 3.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let bool_values = [3_u64, 0, 1, 0];
        let bool_data = bool_values.as_ptr().cast::<u8>();
        let bool_handle = &bool_data as *const *const u8;
        let filter_truthy = CString::new("expr:rb0,rbfiltertruthy:bool-filter-truthy").unwrap();
        let result = call(
            &filter_truthy,
            &[f64::from_bits(bool_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(8).read() }, 1);
        unsafe { libc::free(output.cast_mut().cast()) };
        let empty_string = CString::new("").unwrap();
        let nonempty_string = CString::new("x").unwrap();
        let string_values = [
            3_u64,
            empty_string.as_ptr() as usize as u64,
            nonempty_string.as_ptr() as usize as u64,
            empty_string.as_ptr() as usize as u64,
        ];
        let string_data = string_values.as_ptr().cast::<u8>();
        let string_handle = &string_data as *const *const u8;
        let filter_truthy = CString::new("expr:rs0,rsfiltertruthy:string-filter-truthy").unwrap();
        let result = call(
            &filter_truthy,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { output.add(8).cast::<*const c_char>().read() },
            nonempty_string.as_ptr()
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let find_truthy = CString::new("expr:rs0,rsfindtruthy:string-find-truthy").unwrap();
        assert_eq!(
            call(
                &find_truthy,
                &[f64::from_bits(string_handle as usize as u64)]
            )
            .value
            .to_bits(),
            nonempty_string.as_ptr() as usize as u64
        );
        let bool_some = CString::new("expr:rb0,b1,rbsomeeq:bool-some-equal").unwrap();
        assert_eq!(
            call(
                &bool_some,
                &[f64::from_bits(bool_handle as usize as u64), 1.0]
            )
            .value,
            1.0
        );
        let bool_not = CString::new("expr:rb0,rbmapnot:bool-map-not").unwrap();
        let result = call(&bool_not, &[f64::from_bits(bool_handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(16).cast::<u64>().read() }, 0);
        assert_eq!(unsafe { output.add(24).cast::<u64>().read() }, 1);
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_filter = CString::new("expr:rs0,s1,rsfiltergte:string-filter-gte").unwrap();
        let result = call(
            &string_filter,
            &[
                f64::from_bits(string_handle as usize as u64),
                f64::from_bits(nonempty_string.as_ptr() as usize as u64),
            ],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(
            unsafe { output.add(8).cast::<*const c_char>().read() },
            nonempty_string.as_ptr()
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_identity = CString::new("expr:rs0,rsmapidentity:string-map-identity").unwrap();
        let result = call(
            &string_identity,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(
            unsafe { output.add(16).cast::<*const c_char>().read() },
            nonempty_string.as_ptr()
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_upper = CString::new("expr:rs0,rsmaptouppercase:string-map-uppercase").unwrap();
        let result = call(
            &string_upper,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(16).cast::<*const c_char>().read()).to_bytes() },
            b"X"
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_lengths = CString::new("expr:rs0,rsmaplength:string-map-length").unwrap();
        let result = call(
            &string_lengths,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 0.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 1.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 0.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let string_booleans = CString::new("expr:rs0,rsmaptoboolean:string-map-boolean").unwrap();
        let result = call(
            &string_booleans,
            &[f64::from_bits(string_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<u64>().read() }, 0);
        assert_eq!(unsafe { output.add(16).cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(24).cast::<u64>().read() }, 0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let bool_numbers = CString::new("expr:rb0,rbmaptonumber:boolean-map-number").unwrap();
        let result = call(
            &bool_numbers,
            &[f64::from_bits(bool_handle as usize as u64)],
        );
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 0.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 1.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 0.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let number_strings = CString::new("expr:rn0,rnmaptostring:number-map-string").unwrap();
        let result = call(&number_strings, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(
            unsafe { CStr::from_ptr(output.add(8).cast::<*const c_char>().read()).to_bytes() },
            b"42"
        );
        unsafe { libc::free(output.cast_mut().cast()) };
        let map = CString::new("expr:rn0,c4000000000000000,rnmapmul:array-map").unwrap();
        let result = call(&map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 20.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 40.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 60.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let index_map = CString::new("expr:rn0,rnmapindexadd:array-index-map").unwrap();
        let result = call(&index_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 21.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 32.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let select_map =
            CString::new("expr:rn0,c402e000000000000,rnmapselectgte0:array-select-map").unwrap();
        let result = call(&select_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 15.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 20.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 30.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let branch_map =
            CString::new("expr:rn0,c4034000000000000,rnmapbranch933:array-branch-map").unwrap();
        let result = call(&branch_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 0.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 10.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let remainder =
            CString::new("expr:rn0,c4018000000000000,rnmaprem:array-map-remainder").unwrap();
        let result = call(&remainder, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 4.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 2.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 0.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let reverse_map =
            CString::new("expr:rn0,c4059000000000000,rnmaprsub:array-map-reverse").unwrap();
        let result = call(&reverse_map, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 90.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 80.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, 70.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let negate = CString::new("expr:rn0,rnmapneg:array-map-negate").unwrap();
        let result = call(&negate, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, -10.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, -20.0);
        assert_eq!(unsafe { output.add(24).cast::<f64>().read() }, -30.0);
        unsafe { libc::free(output.cast_mut().cast()) };
        let math_maps: [(&str, extern "C" fn(f64) -> f64); 27] = [
            ("acos", acos_number),
            ("acosh", acosh_number),
            ("asin", asin_number),
            ("asinh", asinh_number),
            ("atan", atan_number),
            ("atanh", atanh_number),
            ("cbrt", cbrt_number),
            ("ceil", ceil_number),
            ("clz32", clz32_number),
            ("cos", cos_number),
            ("cosh", cosh_number),
            ("exp", exp_number),
            ("expm1", expm1_number),
            ("floor", floor_number),
            ("fround", fround_number),
            ("log", log_number),
            ("log1p", log1p_number),
            ("log2", log2_number),
            ("log10", log10_number),
            ("round", round_number),
            ("sign", sign_number),
            ("sin", sin_number),
            ("sinh", sinh_number),
            ("sqrt", square_root_number),
            ("tan", tan_number),
            ("tanh", tanh_number),
            ("trunc", truncate_number),
        ];
        for (method, expected) in math_maps {
            let symbol = CString::new(format!("expr:rn0,rnmap{method}:array-map-math")).unwrap();
            let result = call(&symbol, &[f64::from_bits(handle as usize as u64)]);
            assert!(result.error.is_null(), "{method}");
            let output = (result.value.to_bits() & !ARRAY_RESULT_TAG) as usize as *const u8;
            assert_eq!(
                unsafe { output.add(8).cast::<f64>().read() }.to_bits(),
                expected(10.0).to_bits(),
                "{method}"
            );
            unsafe { libc::free(output.cast_mut().cast()) };
        }
        let empty = [0_u64];
        let empty_data = empty.as_ptr().cast::<u8>();
        let empty_handle = &empty_data as *const *const u8;
        for operation in ["rnreduceadd0", "rnreducerightadd0"] {
            let empty_reduce = CString::new(format!(
                "expr:rn0,c0000000000000000,{operation}:empty-array-reduce"
            ))
            .unwrap();
            assert!(!call(
                &empty_reduce,
                &[f64::from_bits(empty_handle as usize as u64)]
            )
            .error
            .is_null());
        }
        for (operation, expected) in [("rnsomelt", 0.0), ("rneverylt", 1.0)] {
            let symbol = CString::new(format!(
                "expr:rn0,c0000000000000000,{operation}:empty-array-quantifier"
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(empty_handle as usize as u64)]).value,
                expected
            );
        }
        for (operation, expected) in [("rnmin", f64::INFINITY), ("rnmax", f64::NEG_INFINITY)] {
            let symbol = CString::new(format!("expr:rn0,{operation}:empty-extreme")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(empty_handle as usize as u64)]).value,
                expected
            );
        }
        assert_eq!(
            call(&hypot, &[f64::from_bits(empty_handle as usize as u64)]).value,
            0.0
        );
        let zeros = [2_u64, (-0.0f64).to_bits(), 0.0f64.to_bits()];
        let zeros_data = zeros.as_ptr().cast::<u8>();
        let zeros_handle = &zeros_data as *const *const u8;
        for (operation, expected) in [("rnmin", -0.0f64), ("rnmax", 0.0f64)] {
            let symbol = CString::new(format!("expr:rn0,{operation}:zero-extreme")).unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(zeros_handle as usize as u64)])
                    .value
                    .to_bits(),
                expected.to_bits()
            );
        }
        let nan = [2_u64, 1.0f64.to_bits(), f64::NAN.to_bits()];
        let nan_data = nan.as_ptr().cast::<u8>();
        let nan_handle = &nan_data as *const *const u8;
        for operation in ["rnmin", "rnmax"] {
            let symbol = CString::new(format!("expr:rn0,{operation}:nan-extreme")).unwrap();
            assert!(call(&symbol, &[f64::from_bits(nan_handle as usize as u64)])
                .value
                .is_nan());
        }
        let large = [2_u64, 3e200f64.to_bits(), 4e200f64.to_bits()];
        let large_data = large.as_ptr().cast::<u8>();
        let large_handle = &large_data as *const *const u8;
        assert_eq!(
            call(&hypot, &[f64::from_bits(large_handle as usize as u64)]).value,
            0.0f64.hypot(3e200).hypot(4e200)
        );
        let infinite = [2_u64, f64::NAN.to_bits(), f64::INFINITY.to_bits()];
        let infinite_data = infinite.as_ptr().cast::<u8>();
        let infinite_handle = &infinite_data as *const *const u8;
        assert_eq!(
            call(&hypot, &[f64::from_bits(infinite_handle as usize as u64)]).value,
            f64::INFINITY
        );
        let replaced =
            CString::new("expr:rn0,c3ff0000000000000,c4058c00000000000,rnwith:with").unwrap();
        let result = call(&replaced, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 99.0);
        unsafe { libc::free(output.cast()) };
        let sorted = CString::new("expr:rn0,rnsorted:sorted").unwrap();
        let result = call(&sorted, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 10.0);
        unsafe { libc::free(output.cast()) };
        let reverse = CString::new("expr:rn0,arrayreversed:reverse").unwrap();
        let result = call(&reverse, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 3);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 30.0);
        unsafe { libc::free(output.cast()) };
        let slice =
            CString::new("expr:rn0,c3ff0000000000000,c4000000000000000,arrayslice:slice").unwrap();
        let result = call(&slice, &[f64::from_bits(handle as usize as u64)]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 1);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 20.0);
        unsafe { libc::free(output.cast()) };
        let literal = CString::new(
            "expr:arrayempty,c3ff0000000000000,rnappend,c4000000000000000,rnappend,arrayvalue:literal",
        )
        .unwrap();
        let result = call(&literal, &[]);
        assert!(result.error.is_null());
        let output = result.value.to_bits() as usize as *mut u8;
        assert_eq!(unsafe { output.cast::<u64>().read() }, 2);
        assert_eq!(unsafe { output.add(8).cast::<f64>().read() }, 1.0);
        assert_eq!(unsafe { output.add(16).cast::<f64>().read() }, 2.0);
        unsafe { libc::free(output.cast()) };

        for (expression, args, expected) in [
            ("a0,a1,bor", [4_294_967_297.0, 0.0], 1.0),
            ("a0,a1,band", [f64::NAN, 1.0], 0.0),
            ("a0,a1,bxor", [43.0, 1.0], 42.0),
            ("a0,a1,shl", [1.0, 33.0], 2.0),
            ("a0,a1,shr", [-4.0, 1.0], -2.0),
            ("a0,a1,ushr", [-1.0, 0.0], 4_294_967_295.0),
        ] {
            let symbol = CString::new(format!("expr:{expression}:bitwise")).unwrap();
            assert_eq!(call(&symbol, &args).value, expected);
        }
        let bit_not = CString::new("expr:a0,bnot:bit_not").unwrap();
        assert_eq!(call(&bit_not, &[0.0]).value, -1.0);

        let power = CString::new("expr:a0,a1,pow:power").unwrap();
        assert_eq!(call(&power, &[2.0, 5.0]).value, 32.0);
        assert_eq!(call(&power, &[f64::NAN, 0.0]).value, 1.0);
        assert!(call(&power, &[1.0, f64::INFINITY]).value.is_nan());
        assert!(call(&power, &[-2.0, 0.5]).value.is_nan());
        assert_eq!(
            call(&power, &[-0.0, 3.0]).value.to_bits(),
            (-0.0f64).to_bits()
        );
        assert_eq!(call(&power, &[-0.0, -3.0]).value, f64::NEG_INFINITY);

        for (operation, value, expected) in [
            ("isnan", f64::NAN, 1.0),
            ("isnan", 0.0, 0.0),
            ("isfinite", f64::INFINITY, 0.0),
            ("isfinite", 42.0, 1.0),
            ("isinteger", 42.5, 0.0),
            ("isinteger", -42.0, 1.0),
            ("issafeinteger", 9_007_199_254_740_991.0, 1.0),
            ("issafeinteger", 9_007_199_254_740_992.0, 0.0),
        ] {
            let symbol = CString::new(format!("expr:a0,{operation}:{operation}")).unwrap();
            assert_eq!(call(&symbol, &[value]).value, expected);
        }
    }

    #[test]
    fn persists_typed_globals_per_compiled_symbol() {
        let first = CString::new(
            "expr:c0000000000000000,c0000000000000000,globalget,c3ff0000000000000,+,globalset:first-global",
        )
        .unwrap();
        assert_eq!(call(&first, &[]).value, 1.0);
        assert_eq!(call(&first, &[]).value, 2.0);

        let second = CString::new(
            "expr:c0000000000000000,c0000000000000000,globalget,c3ff0000000000000,+,globalset:second-global",
        )
        .unwrap();
        assert_eq!(call(&second, &[]).value, 1.0);

        let writer = CString::new(
            "expr:c0000000000000000,c0000000000000000,c4014000000000000,globalinit,c3ff0000000000000,+,globalset:shared::write",
        )
        .unwrap();
        let reader =
            CString::new("expr:c0000000000000000,c4014000000000000,globalinit:shared::read")
                .unwrap();
        assert_eq!(call(&writer, &[]).value, 6.0);
        assert_eq!(call(&reader, &[]).value, 6.0);
    }

    #[test]
    fn persists_dynamic_callable_entries_per_module() {
        let writer = CString::new(
            "expr:t72756e,c0000000000000000,c4008000000000000,callableset:callable-table::write",
        )
        .unwrap();
        let reader =
            CString::new("expr:t72756e,c0000000000000000,callableget:callable-table::read")
                .unwrap();
        let missing = CString::new(
            "expr:t6d697373696e67,c0000000000000000,callableget:callable-table::missing",
        )
        .unwrap();
        assert_eq!(call(&writer, &[]).value, 3.0);
        assert_eq!(call(&reader, &[]).value, 3.0);
        assert_eq!(call(&missing, &[]).value, -1.0);
    }

    #[test]
    fn snapshots_and_reassigns_dynamic_callable_entries_in_locals() {
        let symbol = CString::new(
            "expr:t6669727374,c0000000000000000,c0000000000000000,callableset,drop,t7365636f6e64,c0000000000000000,c3ff0000000000000,callableset,drop,t6669727374,c0000000000000000,callableget,t7365636f6e64,c0000000000000000,callableget,setl0,t7365636f6e64,c0000000000000000,c0000000000000000,callableset,drop,ln0,nip:callable-snapshot",
        )
        .unwrap();
        let result = call(&symbol, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 1.0);

        let full = CString::new(
            "expr:t6669727374,c0000000000000000,c0000000000000000,asbool,if,c3ff0000000000000,else,c0000000000000000,end,callableset,drop,c0000000000000000,drop,t7365636f6e64,c0000000000000000,c3ff0000000000000,asbool,if,c3ff0000000000000,else,c0000000000000000,end,callableset,drop,c0000000000000000,drop,t6669727374,dup,c0000000000000000,callableget,dup,cbff0000000000000,!=,if,dup,else,dup2,drop,dup,t72756e,strcmp,c0000000000000000,==,if,c0000000000000000,else,cbff0000000000000,end,nip,end,nip,nip,t7365636f6e64,dup,c0000000000000000,callableget,dup,cbff0000000000000,!=,if,dup,else,dup2,drop,dup,t72756e,strcmp,c0000000000000000,==,if,c0000000000000000,else,cbff0000000000000,end,nip,end,nip,nip,setl0,t7365636f6e64,c0000000000000000,c0000000000000000,asbool,if,c3ff0000000000000,else,c0000000000000000,end,callableset,drop,c0000000000000000,drop,ln0,c0000000000000000,==,if,c4010000000000000,c3ff0000000000000,+,else,ln0,c3ff0000000000000,==,if,c4010000000000000,c4000000000000000,*,else,missingcalln,end,end,nip:callable-full-snapshot",
        )
        .unwrap();
        let result = call(&full, &[]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 8.0);

        let arguments = CString::new(
            full.to_str()
                .unwrap()
                .replace("t6669727374", "s0")
                .replace("t7365636f6e64", "s1")
                .replace("c4010000000000000", "a2")
                .replace("callable-full-snapshot", "callable-argument-snapshot"),
        )
        .unwrap();
        let first = CString::new("first").unwrap();
        let second = CString::new("second").unwrap();
        let result = call(
            &arguments,
            &[
                f64::from_bits(first.as_ptr() as usize as u64),
                f64::from_bits(second.as_ptr() as usize as u64),
                4.0,
            ],
        );
        assert!(result.error.is_null());
        assert_eq!(result.value, 8.0);
    }

    #[test]
    fn converts_primitives_to_arena_strings() {
        for (symbol, argument, expected) in [
            ("expr:a0,numstr:number-string", 42.0, "42"),
            ("expr:b0,boolstr:boolean-string", 1.0, "true"),
            ("expr:b0,boolstr:false-string", 0.0, "false"),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[argument]);
            assert!(result.error.is_null());
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                expected
            );
        }
        let symbol = CString::new("expr:s0,strnum:string-number").unwrap();
        let result = call(&symbol, &[1.0]);
        assert!(result.error.is_null());
        assert_eq!(result.value, 42.0);
        let symbol = CString::new("expr:s0,strbool:string-boolean").unwrap();
        for (value, expected) in [("", 0.0), ("value", 1.0)] {
            let value = CString::new(value).unwrap();
            let result = call(&symbol, &[f64::from_bits(value.as_ptr() as usize as u64)]);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }
        let float = CString::new("  -12.5px").unwrap();
        let symbol = CString::new("expr:s0,parsefloat:parse-float").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(float.as_ptr() as usize as u64)]).value,
            -12.5
        );
        let integer = CString::new("11").unwrap();
        let symbol = CString::new("expr:s0,c4000000000000000,parseint:parse-int").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(integer.as_ptr() as usize as u64)]).value,
            3.0
        );
        for (operation, value, argument, expected) in [
            ("tofixed", 12.5, 1.0, "12.5"),
            ("toprecision", 12.5, 3.0, "12.5"),
            ("toradix", 255.0, 16.0, "ff"),
            ("toexponential", 12.5, 1.0, "1.3e+1"),
        ] {
            let symbol = CString::new(format!("expr:a0,a1,{operation}:{operation}")).unwrap();
            let result = call(&symbol, &[value, argument]);
            assert!(result.error.is_null());
            let result = result.value.to_bits() as usize as *mut c_char;
            assert_eq!(
                unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
                expected
            );
            unsafe { libc::free(result.cast()) };
        }
        let shortest = CString::new("expr:a0,toexponential0:toexponential0").unwrap();
        let result = call(&shortest, &[12.5]);
        assert!(result.error.is_null());
        let result = result.value.to_bits() as usize as *mut c_char;
        assert_eq!(
            unsafe { CStr::from_ptr(result) }.to_str().unwrap(),
            "1.25e+1"
        );
        unsafe { libc::free(result.cast()) };
        let invalid = CString::new("expr:a0,a1,tofixed:invalid-fixed").unwrap();
        assert!(!call(&invalid, &[1.0, 101.0]).error.is_null());
        let array = [
            3_u64,
            10.0f64.to_bits(),
            20.0f64.to_bits(),
            30.0f64.to_bits(),
        ];
        let data = array.as_ptr().cast::<u8>();
        let handle = &data as *const *const u8;
        let symbol = CString::new("expr:rn0,arraylen:array-length").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            3.0
        );
        let symbol = CString::new("expr:rn0,isarray:is-array").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            1.0
        );
        let symbol = CString::new("expr:a0,isnotarray:is-not-array").unwrap();
        assert_eq!(call(&symbol, &[42.0]).value, 0.0);
        let symbol = CString::new("expr:a0,a1,numsame:number-same-value").unwrap();
        assert_eq!(call(&symbol, &[f64::NAN, f64::NAN]).value, 1.0);
        assert_eq!(call(&symbol, &[0.0, -0.0]).value, 0.0);
        let symbol = CString::new("expr:s0,s1,strsame:string-same-value").unwrap();
        let text = CString::new("hello").unwrap();
        let other_text = CString::new("hello").unwrap();
        assert_eq!(
            call(
                &symbol,
                &[
                    f64::from_bits(text.as_ptr() as usize as u64),
                    f64::from_bits(other_text.as_ptr() as usize as u64),
                ],
            )
            .value,
            1.0
        );
        let symbol = CString::new("expr:rn0,rn1,refsame:reference-same-value").unwrap();
        let array = f64::from_bits(handle as usize as u64);
        assert_eq!(call(&symbol, &[array, array]).value, 1.0);
        for (symbol, expected) in [
            ("expr:a0,typeofnumber:type-of-number", "number"),
            ("expr:b0,typeofboolean:type-of-boolean", "boolean"),
            ("expr:s0,typeofstring:type-of-string", "string"),
            ("expr:rn0,typeofobject:type-of-array", "object"),
        ] {
            let symbol = CString::new(symbol).unwrap();
            let result = call(&symbol, &[array]);
            assert!(result.error.is_null());
            assert_eq!(
                unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                    .to_str()
                    .unwrap(),
                expected
            );
        }
        for (operation, from, expected) in [
            ("rnindexof", 0.0f64, 1.0),
            ("rnincludes", 0.0, 1.0),
            ("rnlastindexof", f64::INFINITY, 1.0),
        ] {
            let symbol = CString::new(format!(
                "expr:rn0,c4034000000000000,c{:016x},{operation}:{operation}",
                from.to_bits()
            ))
            .unwrap();
            assert_eq!(
                call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
                expected
            );
        }
        let symbol = CString::new("expr:rn0,c4000000000000000,rnat:array-at").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            30.0
        );
        let symbol = CString::new("expr:rn0,c4010000000000000,rnat:array-at-missing").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).error,
            ABSENT_STATUS
        );
        let symbol = CString::new("expr:rn0,c3ff0000000000000,rnget:array-get").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).value,
            20.0
        );
        let symbol = CString::new("expr:rn0,cbff0000000000000,rnget:array-get-negative").unwrap();
        assert_eq!(
            call(&symbol, &[f64::from_bits(handle as usize as u64)]).error,
            ABSENT_STATUS
        );
        let symbol = CString::new("expr:rn0,t7c,rnjoin:array-join").unwrap();
        let result = call(&symbol, &[f64::from_bits(handle as usize as u64)]);
        assert_eq!(
            unsafe { CStr::from_ptr(result.value.to_bits() as usize as *const c_char) }
                .to_str()
                .unwrap(),
            "10|20|30"
        );
    }

    #[test]
    fn evaluates_reversed_noncommutative_operations() {
        for (operation, args, expected) in [
            ("rsub", [2.0, 10.0], 8.0),
            ("rdiv", [2.0, 10.0], 5.0),
            ("rrem", [3.0, 10.0], 1.0),
            ("rpow", [3.0, 2.0], 8.0),
        ] {
            let symbol = CString::new(format!("expr:a0,a1,{operation}:{operation}")).unwrap();
            let result = call(&symbol, &args);
            assert!(result.error.is_null());
            assert_eq!(result.value, expected);
        }
    }

    #[test]
    fn propagates_uncaught_typed_throws() {
        for token in [
            "missingcalln",
            "missingcallb",
            "missingcalls",
            "missingcalla",
            "missingcalld",
        ] {
            let result = call(
                &CString::new(format!("expr:{token}:not-callable")).unwrap(),
                &[],
            );
            assert_eq!(
                unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
                "value is not a function"
            );
        }

        for (symbol, expected) in [
            (
                "expr:c4045000000000000,throwoutn,c0000000000000000:number-throw",
                "42",
            ),
            (
                "expr:c3ff0000000000000,asbool,throwoutb,c0000000000000000:boolean-throw",
                "true",
            ),
            (
                "expr:t626f6f6d,throwouts,c0000000000000000:string-throw",
                "boom",
            ),
        ] {
            let result = call(&CString::new(symbol).unwrap(), &[]);
            assert_eq!(
                unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
                expected
            );
        }

        let storage = [
            3_u64,
            10.0_f64.to_bits(),
            20.0_f64.to_bits(),
            30.0_f64.to_bits(),
        ];
        let data = storage.as_ptr().cast::<u8>();
        let handle = &data as *const *const u8;
        let result = call(
            &CString::new("expr:rn0,throwoutrn,c0000000000000000:array-throw").unwrap(),
            &[f64::from_bits(handle as usize as u64)],
        );
        assert_eq!(
            unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
            "10,20,30"
        );

        let result = call(
            &CString::new("expr:dn0,throwoutd,c0000000000000000:dictionary-throw").unwrap(),
            &[0.0],
        );
        assert_eq!(
            unsafe { CStr::from_ptr(result.error) }.to_str().unwrap(),
            "[object Object]"
        );
    }
}
