#[test]
fn typed_arrays_share_arraybuffer_storage_with_offsets() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 32, &mut data, &mut buffer),
            NAPI_OK
        );
        assert!(!data.is_null());
        *(data as *mut u8).add(4) = 42;

        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 6, 3, buffer, 4, &mut view),
            NAPI_OK
        );
        let mut is_view = false;
        let mut is_buffer = false;
        assert_eq!(napi_is_typedarray(env_ptr, view, &mut is_view), NAPI_OK);
        assert_eq!(
            napi_is_arraybuffer(env_ptr, buffer, &mut is_buffer),
            NAPI_OK
        );
        assert!(is_view && is_buffer);

        let mut kind = -1;
        let mut length = 0;
        let mut view_data = ptr::null_mut();
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                &mut kind,
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!(kind, 6);
        assert_eq!(length, 3);
        assert_eq!(offset, 4);
        assert_eq!(backing, buffer);
        assert_eq!(view_data, data.add(4));
        assert_eq!(*(view_data as *const u8), 42);

        assert_eq!(
            napi_create_typedarray(env_ptr, 6, 1, buffer, 2, &mut view),
            NAPI_INVALID_ARG
        );
        assert_eq!(
            napi_create_typedarray(env_ptr, 8, 5, buffer, 0, &mut view),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn typed_array_indexes_apply_kind_specific_conversions() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let numeric_cases = [
            (0, -129.0, 127.0),
            (1, -1.0, 255.0),
            (2, 3.5, 4.0),
            (3, 65_535.0, -1.0),
            (4, -1.0, 65_535.0),
            (5, 4_294_967_295.0, -1.0),
            (6, -1.0, 4_294_967_295.0),
            (7, 1.25, 1.25),
            (8, 1.5, 1.5),
        ];
        for (kind, input, expected) in numeric_cases {
            let mut backing = ptr::null_mut();
            assert_eq!(
                napi_create_arraybuffer(
                    env_ptr,
                    typedarray_element_size(kind).unwrap(),
                    ptr::null_mut(),
                    &mut backing,
                ),
                NAPI_OK
            );
            let mut view = ptr::null_mut();
            assert_eq!(
                napi_create_typedarray(env_ptr, kind, 1, backing, 0, &mut view),
                NAPI_OK
            );
            let input = env.alloc(Value::Number(input));
            assert_eq!(napi_set_element(env_ptr, view, 0, input), NAPI_OK);
            let mut result = ptr::null_mut();
            assert_eq!(napi_get_element(env_ptr, view, 0, &mut result), NAPI_OK);
            assert!(
                matches!(value_ref(result), Ok(Value::Number(value)) if *value == expected),
                "typed array kind {kind} returned an unexpected value"
            );
            let mut deleted = true;
            assert_eq!(napi_delete_element(env_ptr, view, 0, &mut deleted), NAPI_OK);
            assert!(!deleted);
        }

        for (kind, negative, word) in [(9, true, 1_u64), (10, false, u64::MAX)] {
            let mut backing = ptr::null_mut();
            assert_eq!(
                napi_create_arraybuffer(env_ptr, 8, ptr::null_mut(), &mut backing),
                NAPI_OK
            );
            let mut view = ptr::null_mut();
            assert_eq!(
                napi_create_typedarray(env_ptr, kind, 1, backing, 0, &mut view),
                NAPI_OK
            );
            let input = env.alloc(Value::BigInt {
                negative,
                words: vec![word],
            });
            assert_eq!(napi_set_element(env_ptr, view, 0, input), NAPI_OK);
            let mut result = ptr::null_mut();
            assert_eq!(napi_get_element(env_ptr, view, 0, &mut result), NAPI_OK);
            assert!(matches!(
                value_ref(result),
                Ok(Value::BigInt { negative: actual_negative, words })
                    if *actual_negative == negative && words == &[word]
            ));
        }

        let mut data = ptr::null_mut();
        let mut backing = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 8, &mut data, &mut backing),
            NAPI_OK
        );
        let mut offset_view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 4, 2, backing, 2, &mut offset_view),
            NAPI_OK
        );
        let value = env.alloc(Value::Number(513.0));
        assert_eq!(napi_set_element(env_ptr, offset_view, 1, value), NAPI_OK);
        assert_eq!(
            ptr::read_unaligned((data as *const u8).add(4).cast::<u16>()),
            513
        );
        assert_eq!(napi_detach_arraybuffer(env_ptr, backing), NAPI_OK);
        let mut present = true;
        assert_eq!(
            napi_has_element(env_ptr, offset_view, 1, &mut present),
            NAPI_OK
        );
        assert!(!present);
    }
}

#[test]
fn collection_metadata_properties_follow_javascript_descriptors() {
    unsafe {
        fn number(value: NapiValue) -> f64 {
            match unsafe { value_ref(value) }.unwrap() {
                Value::Number(value) => *value,
                _ => panic!("metadata property was not numeric"),
            }
        }

        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut array = ptr::null_mut();
        assert_eq!(
            napi_create_array_with_length(env_ptr, 3, &mut array),
            NAPI_OK
        );
        let mut actual = ptr::null_mut();
        assert_eq!(
            napi_get_named_property(env_ptr, array, c"length".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(number(actual), 3.0);
        let one = env.alloc(Value::Number(1.0));
        assert_eq!(
            napi_set_named_property(env_ptr, array, c"length".as_ptr(), one),
            NAPI_OK
        );
        let mut length = 0;
        assert_eq!(napi_get_array_length(env_ptr, array, &mut length), NAPI_OK);
        assert_eq!(length, 1);

        let length_key = env.alloc(Value::String("length".into()));
        let mut own = false;
        assert_eq!(
            napi_has_own_property(env_ptr, array, length_key, &mut own),
            NAPI_OK
        );
        assert!(own);
        let mut deleted = true;
        assert_eq!(
            napi_delete_property(env_ptr, array, length_key, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);

        let mut data = ptr::null_mut();
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_buffer(env_ptr, 4, &mut data, &mut buffer),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env_ptr, buffer, c"length".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(number(actual), 4.0);
        assert_eq!(
            napi_has_own_property(env_ptr, buffer, length_key, &mut own),
            NAPI_OK
        );
        assert!(!own, "Buffer length is inherited metadata");
        assert_eq!(
            napi_set_named_property(env_ptr, buffer, c"length".as_ptr(), one),
            NAPI_GENERIC_FAILURE
        );

        let mut backing = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 16, &mut data, &mut backing),
            NAPI_OK
        );
        assert_eq!(
            napi_get_named_property(env_ptr, backing, c"byteLength".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(number(actual), 16.0);
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 4, 3, backing, 2, &mut view),
            NAPI_OK
        );
        for (name, expected) in [(c"length", 3.0), (c"byteLength", 6.0), (c"byteOffset", 2.0)] {
            assert_eq!(
                napi_get_named_property(env_ptr, view, name.as_ptr(), &mut actual),
                NAPI_OK
            );
            assert_eq!(number(actual), expected);
        }
        assert_eq!(
            napi_get_named_property(env_ptr, view, c"buffer".as_ptr(), &mut actual),
            NAPI_OK
        );
        assert_eq!(actual, backing);
        assert_eq!(napi_detach_arraybuffer(env_ptr, backing), NAPI_OK);
        for name in [c"length", c"byteLength", c"byteOffset"] {
            assert_eq!(
                napi_get_named_property(env_ptr, view, name.as_ptr(), &mut actual),
                NAPI_OK
            );
            assert_eq!(number(actual), 0.0);
        }
    }
}

#[test]
fn buffers_can_view_arraybuffer_ranges_without_copying() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut array_buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 8, &mut data, &mut array_buffer),
            NAPI_OK
        );
        *(data as *mut u8).add(2) = 7;
        let mut buffer = ptr::null_mut();
        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, array_buffer, 2, 3, &mut buffer,),
            NAPI_OK
        );
        let mut view_data = ptr::null_mut();
        let mut length = 0;
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut view_data, &mut length),
            NAPI_OK
        );
        assert_eq!(view_data, (data as *mut u8).add(2).cast());
        assert_eq!(length, 3);
        *(view_data as *mut u8).add(1) = 9;
        assert_eq!(*(data as *mut u8).add(3), 9);

        let mut kind = -1;
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                buffer,
                &mut kind,
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!((kind, length, backing, offset), (1, 3, array_buffer, 2));

        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, array_buffer, 7, 2, &mut buffer,),
            NAPI_PENDING_EXCEPTION
        );
        assert!(env.exception.take().is_some());
        assert_eq!(napi_detach_arraybuffer(env_ptr, array_buffer), NAPI_OK);
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut view_data, &mut length),
            NAPI_OK
        );
        assert!(view_data.is_null());
        assert_eq!(length, 0);
    }
}

#[test]
fn buffer_indexes_share_backing_memory_and_cannot_be_deleted() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut owned = ptr::null_mut();
        assert_eq!(
            napi_create_buffer(env_ptr, 3, &mut data, &mut owned),
            NAPI_OK
        );
        let negative = env.alloc(Value::Number(-1.0));
        assert_eq!(napi_set_element(env_ptr, owned, 1, negative), NAPI_OK);
        assert_eq!(*(data as *const u8).add(1), 255);
        let mut actual = ptr::null_mut();
        assert_eq!(napi_get_element(env_ptr, owned, 1, &mut actual), NAPI_OK);
        assert!(matches!(value_ref(actual), Ok(Value::Number(255.0))));

        let key = env.alloc(Value::String("2".into()));
        let wrapped = env.alloc(Value::String("258".into()));
        assert_eq!(napi_set_property(env_ptr, owned, key, wrapped), NAPI_OK);
        assert_eq!(*(data as *const u8).add(2), 2);
        let mut deleted = true;
        assert_eq!(
            napi_delete_element(env_ptr, owned, 2, &mut deleted),
            NAPI_OK
        );
        assert!(!deleted);

        let mut external_bytes = [4_u8, 5];
        let mut external = ptr::null_mut();
        assert_eq!(
            napi_create_external_buffer(
                env_ptr,
                external_bytes.len(),
                external_bytes.as_mut_ptr().cast(),
                None,
                ptr::null_mut(),
                &mut external,
            ),
            NAPI_OK
        );
        let seven = env.alloc(Value::Number(7.0));
        assert_eq!(napi_set_element(env_ptr, external, 0, seven), NAPI_OK);
        assert_eq!(external_bytes[0], 7);

        let mut array_buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 4, &mut data, &mut array_buffer),
            NAPI_OK
        );
        let mut view = ptr::null_mut();
        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, array_buffer, 1, 2, &mut view),
            NAPI_OK
        );
        assert_eq!(napi_set_element(env_ptr, view, 1, seven), NAPI_OK);
        assert_eq!(*(data as *const u8).add(2), 7);

        let mut names = ptr::null_mut();
        assert_eq!(
            napi_get_all_property_names(
                env_ptr,
                view,
                NAPI_KEY_OWN_ONLY,
                NAPI_KEY_ALL_PROPERTIES,
                NAPI_KEY_KEEP_NUMBERS,
                &mut names,
            ),
            NAPI_OK
        );
        let Ok(Value::Array(names)) = value_ref(names) else {
            panic!("buffer indexes were not returned as an array");
        };
        assert_eq!(names.len(), 2);
        assert!(matches!(
            value_ref(names[0].unwrap()),
            Ok(Value::Number(0.0))
        ));
        assert!(matches!(
            value_ref(names[1].unwrap()),
            Ok(Value::Number(1.0))
        ));
    }
}

#[test]
fn sharedarraybuffers_back_views_but_cannot_be_detached() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut shared = ptr::null_mut();
        assert_eq!(
            node_api_create_sharedarraybuffer(env_ptr, 16, &mut data, &mut shared),
            NAPI_OK
        );
        assert!(!data.is_null());
        let mut is_shared = false;
        assert_eq!(
            node_api_is_sharedarraybuffer(env_ptr, shared, &mut is_shared),
            NAPI_OK
        );
        assert!(is_shared);
        let mut is_arraybuffer = true;
        assert_eq!(
            napi_is_arraybuffer(env_ptr, shared, &mut is_arraybuffer),
            NAPI_OK
        );
        assert!(!is_arraybuffer);

        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 4, shared, 3, &mut view),
            NAPI_OK
        );
        let mut view_data = ptr::null_mut();
        let mut length = 0;
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                ptr::null_mut(),
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!((length, backing, offset), (4, shared, 3));
        *view_data.cast::<u8>() = 42;
        assert_eq!(*(data as *mut u8).add(3), 42);

        let mut buffer = ptr::null_mut();
        assert_eq!(
            node_api_create_buffer_from_arraybuffer(env_ptr, shared, 2, 5, &mut buffer),
            NAPI_OK
        );
        assert_eq!(
            napi_detach_arraybuffer(env_ptr, shared),
            NAPI_ARRAYBUFFER_EXPECTED
        );
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut view_data, &mut length),
            NAPI_OK
        );
        assert_eq!(length, 5);
    }
}

#[test]
fn external_sharedarraybuffers_share_memory_and_finalize_without_an_env() {
    unsafe {
        EXTERNAL_SHARED_FINALIZED.store(0, Ordering::Release);
        let mut storage = [1_u8, 2, 3, 4];
        let increment = 3_usize;
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut shared = ptr::null_mut();
        assert_eq!(
            node_api_create_external_sharedarraybuffer(
                env_ptr,
                storage.as_mut_ptr().cast(),
                storage.len(),
                Some(external_shared_finalize),
                (&increment as *const usize).cast_mut().cast(),
                &mut shared,
            ),
            NAPI_OK
        );
        let mut is_shared = false;
        assert_eq!(
            node_api_is_sharedarraybuffer(env_ptr, shared, &mut is_shared),
            NAPI_OK
        );
        assert!(is_shared);

        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, storage.len(), shared, 0, &mut view),
            NAPI_OK
        );
        let seven = env.alloc(Value::Number(7.0));
        assert_eq!(napi_set_element(env_ptr, view, 2, seven), NAPI_OK);
        assert_eq!(storage[2], 7);
        assert_eq!(EXTERNAL_SHARED_FINALIZED.load(Ordering::Acquire), 0);
        drop(env);
        assert_eq!(EXTERNAL_SHARED_FINALIZED.load(Ordering::Acquire), 3);
    }
}

#[test]
fn dataviews_allow_unaligned_bounded_arraybuffer_views() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut data = ptr::null_mut();
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 16, &mut data, &mut buffer),
            NAPI_OK
        );
        *(data as *mut u8).add(3) = 99;
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_dataview(env_ptr, 7, buffer, 3, &mut view),
            NAPI_OK
        );
        let mut is_view = false;
        assert_eq!(napi_is_dataview(env_ptr, view, &mut is_view), NAPI_OK);
        assert!(is_view);

        let mut length = 0;
        let mut view_data = ptr::null_mut();
        let mut backing = ptr::null_mut();
        let mut offset = 0;
        assert_eq!(
            napi_get_dataview_info(
                env_ptr,
                view,
                &mut length,
                &mut view_data,
                &mut backing,
                &mut offset,
            ),
            NAPI_OK
        );
        assert_eq!(length, 7);
        assert_eq!(offset, 3);
        assert_eq!(backing, buffer);
        assert_eq!(*(view_data as *const u8), 99);
        assert_eq!(
            napi_create_dataview(env_ptr, 14, buffer, 3, &mut view),
            NAPI_INVALID_ARG
        );
    }
}

#[test]
fn external_buffers_share_memory_and_finalize_once() {
    unsafe {
        EXTERNAL_MEMORY_FINALIZED.store(0, Ordering::Release);
        let mut array_storage = [0_u8; 16];
        let mut buffer_storage = [1_u8, 2, 3, 4];
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;

        let mut array_buffer = ptr::null_mut();
        assert_eq!(
            napi_create_external_arraybuffer(
                env_ptr,
                array_storage.as_mut_ptr().cast(),
                array_storage.len(),
                Some(external_memory_finalize),
                ptr::null_mut(),
                &mut array_buffer,
            ),
            NAPI_OK
        );
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 4, 3, array_buffer, 2, &mut view),
            NAPI_OK
        );
        let mut view_data = ptr::null_mut();
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut view_data,
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
        *(view_data as *mut u8) = 42;
        assert_eq!(array_storage[2], 42);

        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_external_buffer(
                env_ptr,
                buffer_storage.len(),
                buffer_storage.as_mut_ptr().cast(),
                Some(external_memory_finalize),
                ptr::null_mut(),
                &mut buffer,
            ),
            NAPI_OK
        );
        let mut data = ptr::null_mut();
        let mut length = 0;
        assert_eq!(
            napi_get_buffer_info(env_ptr, buffer, &mut data, &mut length),
            NAPI_OK
        );
        assert_eq!(length, 4);
        *(data as *mut u8).add(1) = 9;
        assert_eq!(buffer_storage[1], 9);
        assert_eq!(EXTERNAL_MEMORY_FINALIZED.load(Ordering::Acquire), 0);
        drop(env);
        assert_eq!(EXTERNAL_MEMORY_FINALIZED.load(Ordering::Acquire), 2);
    }
}

#[test]
fn external_memory_adjustments_are_tracked_per_environment() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut adjusted = 0;
        assert_eq!(
            napi_adjust_external_memory(env_ptr, 4096, &mut adjusted),
            NAPI_OK
        );
        assert_eq!(adjusted, 4096);
        assert_eq!(
            napi_adjust_external_memory(env_ptr, -1024, &mut adjusted),
            NAPI_OK
        );
        assert_eq!(adjusted, 3072);
        env.external_memory = i64::MAX;
        assert_eq!(
            napi_adjust_external_memory(env_ptr, 1, &mut adjusted),
            NAPI_GENERIC_FAILURE
        );
        assert_eq!(env.external_memory, i64::MAX);
    }
}

#[test]
fn detached_arraybuffers_zero_existing_views() {
    unsafe {
        let mut env = Env::new();
        let env_ptr: NapiEnv = &mut env;
        let mut buffer = ptr::null_mut();
        assert_eq!(
            napi_create_arraybuffer(env_ptr, 16, ptr::null_mut(), &mut buffer),
            NAPI_OK
        );
        let mut view = ptr::null_mut();
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 8, buffer, 4, &mut view),
            NAPI_OK
        );
        let mut detached = true;
        assert_eq!(
            napi_is_detached_arraybuffer(env_ptr, buffer, &mut detached),
            NAPI_OK
        );
        assert!(!detached);
        assert_eq!(napi_detach_arraybuffer(env_ptr, buffer), NAPI_OK);
        assert_eq!(
            napi_is_detached_arraybuffer(env_ptr, buffer, &mut detached),
            NAPI_OK
        );
        assert!(detached);

        let mut data = ptr::dangling_mut::<c_void>();
        let mut length = usize::MAX;
        assert_eq!(
            napi_get_arraybuffer_info(env_ptr, buffer, &mut data, &mut length),
            NAPI_OK
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        assert_eq!(
            napi_get_typedarray_info(
                env_ptr,
                view,
                ptr::null_mut(),
                &mut length,
                &mut data,
                ptr::null_mut(),
                ptr::null_mut(),
            ),
            NAPI_OK
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        assert_eq!(
            napi_create_typedarray(env_ptr, 1, 0, buffer, 0, &mut view),
            NAPI_INVALID_ARG
        );
    }
}

