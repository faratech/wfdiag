//! Two-call `AppModel` enumeration with aligned storage and bounded retries.
#[cfg(any(windows, test))]
pub(crate) fn query(
    record_size: usize,
    mut call: impl FnMut(&mut u32, *mut u8, &mut u32) -> u32,
) -> Option<(Vec<usize>, u32)> {
    const INSUFFICIENT_BUFFER: u32 = 122;
    if record_size == 0 {
        return None;
    }
    let mut bytes = 0;
    let mut count = 0;
    if call(&mut bytes, std::ptr::null_mut(), &mut count) != INSUFFICIENT_BUFFER {
        return None;
    }
    for _ in 0..3 {
        if bytes == 0 || bytes > 16 * 1024 * 1024 {
            return None;
        }
        let mut buffer = vec![0_usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let allocated = bytes as usize;
        match call(&mut bytes, buffer.as_mut_ptr().cast(), &mut count) {
            0 if bytes as usize <= allocated && count as usize <= allocated / record_size => {
                return Some((buffer, count));
            }
            INSUFFICIENT_BUFFER => {} // graph changed between the two calls
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_size_query_and_retries_graph_growth() {
        let mut calls = 0;
        let (buffer, count) = query(16, |bytes, ptr, count| {
            calls += 1;
            match calls {
                1 => {
                    assert!(ptr.is_null());
                    *bytes = 16;
                    122
                }
                2 => {
                    assert!(!ptr.is_null());
                    *bytes = 32;
                    122
                }
                _ => {
                    *count = 2;
                    0
                }
            }
        })
        .unwrap();
        assert_eq!(count, 2);
        assert_eq!(buffer.len() * size_of::<usize>(), 32);
    }
    #[test]
    fn rejects_unexpected_errors_and_invalid_sizes() {
        assert!(query(16, |_, _, _| 15700).is_none());
        assert!(
            query(16, |size, _, _| {
                *size = u32::MAX;
                122
            })
            .is_none()
        );
    }
}
