#!/usr/bin/env python3
"""Fail-closed, hash-pinned native overlay reconstruction (Python stdlib only)."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tempfile
from typing import Any


MODULE_DIR = Path(__file__).resolve().parent
VALIDATION_BRANCH = "validation/native-return"
BASELINE_COMMIT = "8b353a99d4995c8217b9e73cd308ea85d2d6e5d8"
BASELINE_TREE = "f3dd90940e1d36ce27e57774b62f87b9df4140af"
FORCED_ROOT_PROVIDER_PATHS = [
    "crates/thaw-quickjs/src/quickjs/api.rs",
    "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js",
]
FORCED_ROOT_STAGE_EXTRA_PATHS = [
    "crates/thaw-std/src/json.rs",
    *FORCED_ROOT_PROVIDER_PATHS,
]
DEPENDENCY_SOURCE_PATHS = [
    "crates/thaw-runtime/src/runtime/native_values/strings.rs",
    "crates/thaw-runtime/src/runtime/promises.rs",
    "crates/thaw-runtime/src/tests.rs",
    "crates/thaw-std/src/json.rs",
]
STD_HTTP_PATH = "crates/thaw-std/src/http.rs"
STD_HTTP_BASE_SHA256 = "4ad52d51d87666b1a910477daa3c8b7a2c3b4b9825198a1b1495242aba1cbddf"
STD_HTTP_BASE_GIT_BLOB_SHA1 = "ff3ff9850cef56b5dd7be727c6bdbf74adb143d8"
STD_HTTP_FINAL_SHA256 = "22f6c8809a304857a4fb800ef9949f70cbe573a8a2a4930b3df847640d1435ed"
STD_COMPILE_EXTRA_PATHS = [*FORCED_ROOT_STAGE_EXTRA_PATHS, STD_HTTP_PATH]
STD_COMPILE_SOURCE_PATHS = [STD_HTTP_PATH, "crates/thaw-std/src/json.rs"]
QUICKJS_WASM_PATH = "crates/thaw-quickjs/src/quickjs/wasm.rs"
QUICKJS_WASM_BASE_SHA256 = "6eabef4cc98d6d123f8c8772f3b534ad06682e5e0792f6d18e56a350558f2a56"
QUICKJS_WASM_BASE_GIT_BLOB_SHA1 = "8735b64671f7b03806397986e9fbbe08d08ac747"
QUICKJS_WASM_FINAL_SHA256 = "6982e1394e32a2ddd083d3f5c31c9e8cf1f5e8b41105722c7cd4826efb73c0db"
QUICKJS_TESTS_PATH = "crates/thaw-quickjs/src/tests.rs"
QUICKJS_TESTS_BASE_SHA256 = "f9fd787016d9dcf3ec30aa52a0ff23c8f1bc970d88bbac375c0e4d65634992c6"
QUICKJS_TESTS_BASE_GIT_BLOB_SHA1 = "a146b07ee0b1d398e3abb9a51e58a6f3bbce607a"
QUICKJS_TESTS_FINAL_SHA256 = "c49ffdf065c4fd7d2d5765affed1daf773c07ef6b46eca137fa3c25ec1d5cab1"
QUICKJS_RUNTIME_EXTRA_PATHS = [*STD_COMPILE_EXTRA_PATHS, QUICKJS_WASM_PATH]
QUICKJS_RUNTIME_SOURCE_PATHS = [
    "crates/thaw-quickjs/src/quickjs/api.rs",
    QUICKJS_WASM_PATH,
    "crates/thaw-runtime/src/runtime/native_values/numbers.rs",
    "crates/thaw-runtime/src/runtime/native_values/template_strings.rs",
    "crates/thaw-runtime/src/tests.rs",
]
QUICKJS_API_FOLLOWUP_EXTRA_PATHS = [*QUICKJS_RUNTIME_EXTRA_PATHS, QUICKJS_TESTS_PATH]
QUICKJS_API_FOLLOWUP_SOURCE_PATHS = [
    "crates/thaw-quickjs/src/quickjs/api.rs",
    QUICKJS_WASM_PATH,
    QUICKJS_TESTS_PATH,
]
EXPECTED: dict[str, Any] = {
    "schema_version": 1,
    "validation_branch": VALIDATION_BRANCH,
    "baseline": {"commit": BASELINE_COMMIT, "tree": BASELINE_TREE},
    "source_files": {
        "handoff_manifest": {
            "path": "docs/codebase-refactor-handoff/manifest.json",
            "sha256": "e0c9d123c95e616576264708fff73bd91d23e2b83cee8281256d5aab617db5c9",
        },
        "original_patch": {
            "path": "docs/codebase-refactor-handoff/drafts/native/candidate.patch",
            "sha256": "359172c41195eb98f687f5f36d4ed9a07d8d5ff38e5e3564ce609552994a6506",
        },
        "scope_patch": {
            "path": "docs/codebase-refactor-handoff/drafts/native/scope-boundary-continuation-20261005/delta.patch",
            "sha256": "1387f9a75e443e2030656ec7d4a3b2eb8f1d89573dc0e02c32f881b2b4b50e22",
        },
        "cargo_lock": {
            "path": "Cargo.lock",
            "sha256": "75b3cf821bfbde27d92f80d66b4db1accf368dffc0310375b986dc28d4578025",
        },
    },
    "payload_patches": {
        "discard-v2.patch": "0ffe9fcec530feb12a2ca084399643861c240cc81906c9aab4677cc529aceb39",
        "return-v1.patch": "2e2e2057925542c0790d5ec879ce94d55e3ba5798b470aa5cd85327821fd1ec8",
        "compile-repairs-v1.patch": "2994f62e98a4fb9cbd0b792b20da09bf505874d749447bf8f0e43776d25e0f05",
        "cumulative-hir-v1.patch": "63744b75a96ce30ff503d90379b08ec20ffbebb5f5f68716972d6c7d6f17f368",
        "next-hir-v1.patch": "c0d4addaf22a7c7c26a133153b8298cc39013794f0263228468dddb8e0f3936b",
        "forced-root-wire-v1.patch": "a7f04af1afa5b9d41d7a8dca984ccefba020cdd36f19175ef14a8f809a6e8683",
        "dependency-compile-v1.patch": "68a5c8e397df3462aeb83879d854837a423ef297d523f455ecd62d05b7281aea",
        "std-compile-v1.patch": "25263410104929b361693cf228056514e27a41c262f8680c3e7bfb4a0c09cc18",
    },
    "artifacts": {
        "original-native.sha256": "4e219dde2129cf5876497edd6f9032cebdbee89eded9cce20e50ecda041053ce",
        "scope-v5.sha256": "227274cad19f045e158168cc2b341744fb62e2d0e78c633ece4703e462d7a8e3",
        "discard-v2.sha256": "c47d4ab7e4a805190e719b13b99bd7d233e24a5007afcbff6becebab9493e328",
        "return-v1.sha256": "51b1d9dd9a501538cc44fcaf8bfa72cc5a3f9ceb34c2942d4a0c676acc2e7031",
        "native-base.sha256": "da86c2c16619de2e236cdd50b9d2572465fbdd4dc3b75b680cf91c6b0f6bdb72",
        "net-owners.txt": "e6e1b5beb336d919941e914ddc7569c3b5567ed30344f69adde12c4f1f891d0c",
        "scope-v5-test-plan.md": "9eb740f8f418de5e7f8c7cd88607d5b269c7495a34600d937bf8653f7e55ae81",
        "compile-repairs-v1-base.sha256": "88bb3556306a13f8148d229b88057ed4324f9ac43861cbc5d2b3fd52df03e80e",
        "compile-repairs-v1.sha256": "d3b7c0e06ca053f4b342dcbc8ac1247a3460c946a35a2b22d2645c76251a2aa9",
        "cumulative-hir-v1-base.sha256": "d3b7c0e06ca053f4b342dcbc8ac1247a3460c946a35a2b22d2645c76251a2aa9",
        "cumulative-hir-v1.sha256": "0be3783d6241706ba2dd492704162aa9e0076dcc4a1de96e309eca9b512ba81c",
        "next-hir-v1-base.sha256": "0be3783d6241706ba2dd492704162aa9e0076dcc4a1de96e309eca9b512ba81c",
        "next-hir-v1.sha256": "f8915460efcaafc82a4392d9ca6b656dc150045d5cad4458b66755ef5705635b",
        "forced-root-wire-v1-base.sha256": "23da8386704a188e890bd7e591bb395ce611d45e8fbf0ad264716b5d61e3c9bf",
        "forced-root-wire-v1.sha256": "83a99e4934c4108992f874ca75ff2ea366283fbc3c543afaf9fee6489cb95941",
        "dependency-compile-v1-base.sha256": "83a99e4934c4108992f874ca75ff2ea366283fbc3c543afaf9fee6489cb95941",
        "dependency-compile-v1.sha256": "605121c9db1c995897770fc223d6207d18e9a6075e11725c2c9c0e25f8e2288d",
        "std-compile-v1-base.sha256": "cc7f08e15509afde400423c709a28703152a2e20df1d77518dc1fcd58ef4fc32",
        "std-compile-v1.sha256": "2f0b3a27d30ba1e294d760e53ac226e59b3e375ee3ca0bb8668f0ef8b197fec3",
    },
    "stage_manifests": [
        {"name": "original-native", "file": "original-native.sha256", "sha256": "4e219dde2129cf5876497edd6f9032cebdbee89eded9cce20e50ecda041053ce", "count": 175},
        {"name": "scope-v5", "file": "scope-v5.sha256", "sha256": "227274cad19f045e158168cc2b341744fb62e2d0e78c633ece4703e462d7a8e3", "count": 176},
        {"name": "discard-v2", "file": "discard-v2.sha256", "sha256": "c47d4ab7e4a805190e719b13b99bd7d233e24a5007afcbff6becebab9493e328", "count": 179},
        {"name": "return-v1", "file": "return-v1.sha256", "sha256": "51b1d9dd9a501538cc44fcaf8bfa72cc5a3f9ceb34c2942d4a0c676acc2e7031", "count": 180},
        {"name": "compile-repairs-v1", "file": "compile-repairs-v1.sha256", "sha256": "d3b7c0e06ca053f4b342dcbc8ac1247a3460c946a35a2b22d2645c76251a2aa9", "count": 181, "extra_paths": ["crates/thaw-std/src/json.rs"], "path_order": "lexical"},
        {"name": "cumulative-hir-v1", "file": "cumulative-hir-v1.sha256", "sha256": "0be3783d6241706ba2dd492704162aa9e0076dcc4a1de96e309eca9b512ba81c", "count": 181, "extra_paths": ["crates/thaw-std/src/json.rs"], "path_order": "lexical"},
        {"name": "next-hir-v1", "file": "next-hir-v1.sha256", "sha256": "f8915460efcaafc82a4392d9ca6b656dc150045d5cad4458b66755ef5705635b", "count": 181, "extra_paths": ["crates/thaw-std/src/json.rs"], "path_order": "lexical"},
        {"name": "forced-root-wire-v1", "file": "forced-root-wire-v1.sha256", "sha256": "83a99e4934c4108992f874ca75ff2ea366283fbc3c543afaf9fee6489cb95941", "count": 183, "extra_paths": ["crates/thaw-std/src/json.rs", "crates/thaw-quickjs/src/quickjs/api.rs", "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js"], "path_order": "lexical"},
        {"name": "dependency-compile-v1", "file": "dependency-compile-v1.sha256", "sha256": "605121c9db1c995897770fc223d6207d18e9a6075e11725c2c9c0e25f8e2288d", "count": 183, "extra_paths": ["crates/thaw-std/src/json.rs", "crates/thaw-quickjs/src/quickjs/api.rs", "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js"], "path_order": "lexical"},
        {"name": "std-compile-v1", "file": "std-compile-v1.sha256", "sha256": "2f0b3a27d30ba1e294d760e53ac226e59b3e375ee3ca0bb8668f0ef8b197fec3", "count": 184, "extra_paths": STD_COMPILE_EXTRA_PATHS, "path_order": "lexical"},
    ],
    "repair_base": {
        "file": "compile-repairs-v1-base.sha256",
        "sha256": "88bb3556306a13f8148d229b88057ed4324f9ac43861cbc5d2b3fd52df03e80e",
        "count": 181,
        "path_order": "components",
        "extra_owner": {
            "path": "crates/thaw-std/src/json.rs",
            "sha256": "2844a99da40572ae069bfc37cd71f620a6c9a22116bece38862ae552273948b6",
            "git_blob_sha1": "088b48b46641ca9beda9e1847e86a0ad47faaf5b",
            "final_sha256": "3e4873700556b7bbc35b3d54e09d5a761f66119a0651c453923489254ee18dca",
        },
    },
    "cumulative_base": {
        "file": "cumulative-hir-v1-base.sha256",
        "sha256": "d3b7c0e06ca053f4b342dcbc8ac1247a3460c946a35a2b22d2645c76251a2aa9",
        "count": 181,
        "path_order": "lexical",
        "paired_after": "compile-repairs-v1",
    },
    "next_hir_base": {
        "file": "next-hir-v1-base.sha256",
        "sha256": "0be3783d6241706ba2dd492704162aa9e0076dcc4a1de96e309eca9b512ba81c",
        "count": 181,
        "path_order": "lexical",
        "paired_after": "cumulative-hir-v1",
    },
    "forced_root_wire_base": {
        "file": "forced-root-wire-v1-base.sha256",
        "sha256": "23da8386704a188e890bd7e591bb395ce611d45e8fbf0ad264716b5d61e3c9bf",
        "count": 183,
        "path_order": "lexical",
        "paired_after": "next-hir-v1",
        "extra_owners": [
            {
                "path": "crates/thaw-quickjs/src/quickjs/api.rs",
                "sha256": "02907f79127e23ede4faea4fd89e4ac466cde76d5f4febc8831fa8b5df2d6d36",
                "git_blob_sha1": "4ade648dbe5203bff00014bd7903821d595680b0",
                "final_sha256": "be84b1a2c295c8bcc62ec41427999d75f485b90b4711e8b384424782f9e1bbf9",
            },
            {
                "path": "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js",
                "sha256": "0a223f6a0934dfddba0217e342f7ed1f09242b806d313f4da26eca012cd7ee3a",
                "git_blob_sha1": "73eeee5f1df8a84b65fd5b088c374aa0a6e1521e",
                "final_sha256": "bdca00a406d512f19349d41b5ac70d145a28eb0d029c5521dd9c375a68e50a81",
            },
        ],
    },
    "dependency_compile_base": {
        "file": "dependency-compile-v1-base.sha256",
        "sha256": "83a99e4934c4108992f874ca75ff2ea366283fbc3c543afaf9fee6489cb95941",
        "count": 183,
        "path_order": "lexical",
        "paired_after": "forced-root-wire-v1",
    },
    "std_compile_base": {
        "file": "std-compile-v1-base.sha256",
        "sha256": "cc7f08e15509afde400423c709a28703152a2e20df1d77518dc1fcd58ef4fc32",
        "count": 184,
        "path_order": "lexical",
        "paired_after": "dependency-compile-v1",
        "extra_owner": {
            "path": STD_HTTP_PATH,
            "sha256": STD_HTTP_BASE_SHA256,
            "git_blob_sha1": STD_HTTP_BASE_GIT_BLOB_SHA1,
            "final_sha256": STD_HTTP_FINAL_SHA256,
        },
    },
    "native_base": {"file": "native-base.sha256", "sha256": "da86c2c16619de2e236cdd50b9d2572465fbdd4dc3b75b680cf91c6b0f6bdb72", "count": 24},
    "net_owners": {"file": "net-owners.txt", "sha256": "e6e1b5beb336d919941e914ddc7569c3b5567ed30344f69adde12c4f1f891d0c", "count": 58, "modified": 53, "new": 5},
    "patch_order": ["original-native", "scope-v5", "discard-v2", "return-v1", "compile-repairs-v1", "cumulative-hir-v1", "next-hir-v1", "forced-root-wire-v1", "dependency-compile-v1", "std-compile-v1"],
    "subset_roots": [
        ".gitignore",
        "Cargo.lock",
        "Cargo.toml",
        "crates/thaw-arena",
        "crates/thaw-hir",
        "crates/thaw-llvm",
        "crates/thaw-runtime",
    ],
    "control_owners": [
        "crates/thaw-llvm/src/hir_codegen/tests/async/eval_then_returns.rs",
        "crates/thaw-llvm/src/hir_codegen/tests/async/resolver_provenance.rs",
        "crates/thaw-llvm/src/hir_codegen/tests/async/union_discard.rs",
        "crates/thaw-llvm/src/hir_codegen/tests/async/union_discard_eval_then.rs",
        "crates/thaw-llvm/src/hir_codegen/tests/async/union_discard_jit.rs",
    ],
    "required_includes": {
        "crates/thaw-llvm/src/hir_codegen/tests/async.rs": [
            "async/syntax.rs",
            "async/callbacks.rs",
            "async/promises.rs",
            "async/resolver_provenance.rs",
            "async/expressions.rs",
            "async/control_flow.rs",
            "async/http_lambda.rs",
            "async/union_discard.rs",
            "async/union_discard_jit.rs",
            "async/union_discard_eval_then.rs",
            "async/eval_then_returns.rs",
        ]
    },
    "runner_dependencies": {
        "llvm_asset": {
            "filename": "LLVM-22.1.8-Linux-X64.tar.xz",
            "url": "https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.8/LLVM-22.1.8-Linux-X64.tar.xz",
            "sha256": "df0e1ecf16caf3489a272a5eea4eec9b0d82878f6477fa309504f918a0006384",
        },
        "system_packages": ["libuv1-dev"],
        "llvm_prefix": "/opt/llvm-22",
        "cargo_lock_sha256": "75b3cf821bfbde27d92f80d66b4db1accf368dffc0310375b986dc28d4578025",
        "inkwell_version": "0.10.0",
        "llvm_sys_version": "221.0.1",
        "inkwell_feature": "llvm22-1",
    },
    "test_filters": [
        {"package": "thaw-llvm", "filter": "native_eval_then_return_"},
        {"package": "thaw-llvm", "filter": "discarded_native_promise_union_"},
        {"package": "thaw-llvm", "filter": "discarded_promise_results_release_only_the_selected_owned_value"},
        {"package": "thaw-llvm", "filter": "named_and_closure_returns_retain_borrowed_promises"},
        {"package": "thaw-llvm", "filter": "native_promise_scope_boundaries_codegen_regression_control"},
        {"package": "thaw-llvm", "filter": "native_promise_scope_tables_restore_after_codegen_errors"},
        {"package": "thaw-llvm", "filter": "reactive_preheader_promotions_preserve_exact_scope_and_successor_context"},
        {"package": "thaw-llvm", "filter": "reactive_preheader_codegen_error_restores_nonempty_scope_exactly"},
        {"package": "thaw-llvm", "filter": "stack_owner_live_merge_preserves_exact_branch_bindings"},
        {"package": "thaw-llvm", "filter": "hir_if_live_arm_keeps_sibling_stack_binding_and_runtime_flag"},
        {"package": "thaw-llvm", "filter": "native_promise_exception_descriptor_survives_all_cleanup_handoffs"},
        {"package": "thaw-llvm", "filter": "nested_codegen_scope_contexts_restore_exact_nonempty_state"},
        {"package": "thaw-llvm", "filter": "compile_lambda_restores_scope_after_real_inner_body_error"},
        {"package": "thaw-llvm", "filter": "compile_async_lambda_restores_scope_after_real_inner_body_error"},
        {"package": "thaw-llvm", "filter": "published_throw_keeps_fresh_native_exception_descriptor"},
        {"package": "thaw-llvm", "filter": "text_only_throw_clears_stale_native_exception_descriptor"},
        {"package": "thaw-llvm", "filter": "blocking_and_async_exception_handoffs_copy_native_descriptor_before_release"},
        {"package": "thaw-llvm", "filter": "plain_string_resolver_clears_native_descriptor_from_real_typed_publisher"},
        {"package": "thaw-llvm", "filter": "typed_native_reason_resolver_preserves_its_published_descriptor"},
        {"package": "thaw-llvm", "filter": "pending_rethrow_keeps_text_and_descriptor_on_their_own_channels"},
        {"package": "thaw-hir", "filter": "lowers_try_catch"},
        {"package": "thaw-hir", "filter": "finally_separates_text_only_throws_from_fresh_published_tuples"},
        {"package": "thaw-hir", "filter": "finally_snapshots_return_and_throw_values_before_mutation"},
        {"package": "thaw-hir", "filter": "thaw_remaining_"},
        {"package": "thaw-std", "filter": "typed_decode_scope_tracks_dynamic_retains_merges_and_excludes_reentry"},
        {"package": "thaw-hir", "filter": "thaw_binding_helper_"},
        {"package": "thaw-hir", "filter": "receiver_pattern_inference_"},
        {"package": "thaw-hir", "filter": "thaw_rethrow_cleanup_"},
        {"package": "thaw-hir", "filter": "error_argument_staging_"},
        {"package": "thaw-hir", "filter": "existing_native_spread_staging_mode_false_is_unchanged"},
        {"package": "thaw-quickjs", "filter": "forced_root_"},
        {"package": "thaw-quickjs", "filter": "graph_codec_roundtrip_preserves_negative_zero"},
        {"package": "thaw-quickjs", "filter": "graph_codec_roundtrip_retains_identity_and_releases_live_lease"},
        {"package": "thaw-quickjs", "filter": "graph_codec_uses_bootstrap_intrinsics_after_global_replacement"},
        {"package": "thaw-quickjs", "filter": "handle_registry_identity_ignores_later_object_is_override"},
        {"package": "thaw-quickjs", "filter": "failed_exception_graph_grant_retires_producer_handle_lease"},
        {"package": "thaw-quickjs", "filter": "exact_mixed_pre_dispatch_consumes_registered_graph_grant_once"},
        {"package": "thaw-std", "filter": "unregistered_graph_wire_cannot_transfer_napi_lease_tokens"},
        {"package": "thaw-std", "filter": "mutated_graph_wire_retires_only_registered_snapshot_leases"},
        {"package": "thaw-runtime", "filter": "tests::dependency_compile_uri_byte_iteration"},
        {"package": "thaw-runtime", "filter": "tests::dependency_compile_root_locale_case"},
        {"package": "thaw-runtime", "filter": "tests::dependency_compile_finally_adopt_deferred_frame"},
        {"package": "thaw-std", "filter": "json::tests::dependency_compile_writable_expression"},
        {"package": "thaw-std", "filter": "json::tests::dependency_compile_as_string_variants"},
        {"package": "thaw-runtime", "filter": "tests::private_exception_provenance_has_stable_nine_word_layout"},
        {"package": "thaw-runtime", "filter": "tests::finally_adopt_roots_original_result_object_and_aggregate_until_output_owns_them"},
        {"package": "thaw-runtime", "filter": "tests::purged_aggregate_callbacks_ignore_late_child_results_and_release_roots"},
        {"package": "thaw-std", "filter": "json::tests::jit_dictionary_callbacks_preserve_presence_and_typed_failures"},
        {"package": "thaw-std", "filter": "json::tests::a_missing_key_or_index_is_distinguishable_from_an_explicit_null"},
        {"package": "thaw-std", "filter": "http::tests::peer_send_eof_preserves_pending_streamed_response"},
    ],
    "expected_test_names": {
        "thaw_remaining_": [
            "thaw_remaining_program_error_abi_mutator_reaches_neighboring_ffi_call",
            "thaw_remaining_program_ownership_mutator_reaches_neighboring_ffi_call",
            "thaw_remaining_program_string_abi_mutator_reaches_neighboring_ffi_call",
            "thaw_remaining_function_ref_this_contributes_no_captured_names",
            "thaw_remaining_lambda_uses_declared_captures_without_scanning_its_body",
            "thaw_remaining_function_ref_this_has_no_binding_or_await_facts",
            "thaw_remaining_bytes_erasure_visits_this_signature_and_callable_reference_types",
            "thaw_remaining_native_exception_is_preserved_as_an_opaque_type_leaf",
        ],
        "typed_decode_scope_tracks_dynamic_retains_merges_and_excludes_reentry": [
            "typed_decode_scope_tracks_dynamic_retains_merges_and_excludes_reentry",
        ],
        "thaw_binding_helper_": [
            "thaw_binding_helper_declaration_flags_follow_resolved_symbols",
            "thaw_binding_helper_iteration_cell_inherits_only_current_outer_immutability",
        ],
        "receiver_pattern_inference_": [
            "receiver_pattern_inference_parsed_annotations_keep_pattern_and_legacy_states",
            "receiver_pattern_inference_declarations_remain_accepted_without_calls",
            "receiver_pattern_inference_source_negative_and_shape_controls",
            "receiver_pattern_inference_physical_and_split_modes_match_receiver_first",
            "receiver_pattern_inference_synthetic_and_contextual_modes_stay_separate",
            "receiver_pattern_inference_distinguishes_absent_undefined_and_legacy_receivers",
            "receiver_pattern_inference_optional_rest_indices_and_key_literals_stay_visible_only",
            "receiver_pattern_inference_implicit_fallbacks_follow_actual_match_order",
            "receiver_pattern_inference_explicit_constraints_precede_actual_mismatch",
            "receiver_pattern_inference_type_only_and_receiver_free_promise_controls",
        ],
        "thaw_rethrow_cleanup_": [
            "thaw_rethrow_cleanup_named_suffix_catches_publish_the_visible_carrier",
            "thaw_rethrow_cleanup_unannotated_alias_keeps_and_publishes_the_carrier",
            "thaw_rethrow_cleanup_mutation_retags_and_publishes_the_current_binding",
            "thaw_rethrow_cleanup_nested_shadow_publishes_the_outer_carrier_alias",
            "thaw_rethrow_cleanup_number_typeof_narrowing_uses_member_zero_f64",
            "thaw_rethrow_cleanup_common_publisher_checks_all_carrier_arms_structurally",
            "thaw_rethrow_cleanup_member_six_routes_original_pointer_to_object_setter",
        ],
        "error_argument_staging_": [
            "error_argument_staging_keeps_all_raw_values_once",
            "error_argument_staging_flattens_literal_spreads_in_order",
            "error_argument_staging_snapshots_tuple_members_before_next_argument",
            "error_argument_staging_evaluates_empty_tuple_and_extras",
            "error_argument_staging_preserves_reference_and_union_representations",
            "error_argument_staging_wrapper_precedes_message_work",
            "error_argument_staging_retains_existing_spread_admission",
        ],
        "existing_native_spread_staging_mode_false_is_unchanged": [
            "existing_native_spread_staging_mode_false_is_unchanged",
        ],
    },
    "expected_full_test_names": {
        "forced_root_": [
            "forced_root_graph_wire_controls::forced_root_transfer_failure_rolls_back_once",
            "forced_root_graph_wire_controls::forced_root_decoded_lease_outlives_consumed_input_and_root",
            "forced_root_graph_wire_controls::forced_root_guard_preserves_other_graph_modes",
            "forced_root_graph_wire_controls::forced_root_default_query_boundary_is_explicit",
            "forced_root_graph_wire_controls::forced_root_capture_packet_uses_existing_node_grammar",
            "forced_root_graph_wire_controls::forced_root_capture_preserves_identity_aliases_and_liveness",
            "forced_root_graph_wire_controls::forced_root_capture_performs_no_original_value_hooks",
            "forced_root_graph_wire_controls::forced_root_packet_ignores_replaced_intrinsics_and_metadata_hooks",
        ],
        "graph_codec_roundtrip_preserves_negative_zero": ["graph_codec_roundtrip_preserves_negative_zero"],
        "graph_codec_roundtrip_retains_identity_and_releases_live_lease": ["graph_codec_roundtrip_retains_identity_and_releases_live_lease"],
        "graph_codec_uses_bootstrap_intrinsics_after_global_replacement": ["graph_codec_uses_bootstrap_intrinsics_after_global_replacement"],
        "handle_registry_identity_ignores_later_object_is_override": ["handle_registry_identity_ignores_later_object_is_override"],
        "failed_exception_graph_grant_retires_producer_handle_lease": ["failed_exception_graph_grant_retires_producer_handle_lease"],
        "exact_mixed_pre_dispatch_consumes_registered_graph_grant_once": ["exact_mixed_pre_dispatch_consumes_registered_graph_grant_once"],
        "unregistered_graph_wire_cannot_transfer_napi_lease_tokens": ["json::unregistered_graph_wire_cannot_transfer_napi_lease_tokens"],
        "mutated_graph_wire_retires_only_registered_snapshot_leases": ["json::mutated_graph_wire_retires_only_registered_snapshot_leases"],
        "tests::dependency_compile_uri_byte_iteration": ["tests::dependency_compile_uri_byte_iteration"],
        "tests::dependency_compile_root_locale_case": ["tests::dependency_compile_root_locale_case"],
        "tests::dependency_compile_finally_adopt_deferred_frame": ["tests::dependency_compile_finally_adopt_deferred_frame"],
        "json::tests::dependency_compile_writable_expression": ["json::tests::dependency_compile_writable_expression"],
        "json::tests::dependency_compile_as_string_variants": ["json::tests::dependency_compile_as_string_variants"],
        "tests::private_exception_provenance_has_stable_nine_word_layout": ["tests::private_exception_provenance_has_stable_nine_word_layout"],
        "tests::finally_adopt_roots_original_result_object_and_aggregate_until_output_owns_them": ["tests::finally_adopt_roots_original_result_object_and_aggregate_until_output_owns_them"],
        "tests::purged_aggregate_callbacks_ignore_late_child_results_and_release_roots": ["tests::purged_aggregate_callbacks_ignore_late_child_results_and_release_roots"],
        "json::tests::jit_dictionary_callbacks_preserve_presence_and_typed_failures": ["json::tests::jit_dictionary_callbacks_preserve_presence_and_typed_failures"],
        "json::tests::a_missing_key_or_index_is_distinguishable_from_an_explicit_null": ["json::tests::a_missing_key_or_index_is_distinguishable_from_an_explicit_null"],
        "http::tests::peer_send_eof_preserves_pending_streamed_response": ["http::tests::peer_send_eof_preserves_pending_streamed_response"],
    },
    # Informational fingerprints only; their underlying prose is not shipped or validated by CI.
    "provenance_only_review_fingerprints": {
        "scope_v5": "ef1073a1d8204c08d2caa36611d3a02a8f34482cf7299f965fd13fe634e24d09",
        "discard_v2": "91f328cce86a8102e53045be4069b646487ea0acf48e141a11099197942571fd",
        "return_v1": "b0935bbc26b8aaf0e4c20c4778660cd1729ba349a827d768fe5fff38dc7e018a",
    },
}

EXPECTED["payload_patches"]["quickjs-runtime-compile-v1.patch"] = "ab6405f01b6d8377b7d9282edf90a14003ad762b5dfc0cc5a8d7252db98f9a50"
EXPECTED["artifacts"].update({
    "quickjs-runtime-compile-v1-base.sha256": "2bd5d6ebe24b70d971ba1e9fbaee069562c333a075f48ff3097d11cf541127a2",
    "quickjs-runtime-compile-v1.sha256": "15614690c79ade16025fded42bc9905fb9aef1add904c6280ef6538f67979538",
    "net-owners.txt": "50d21ac22ff6888bd58eee813964c5a843e4a476620e480482cb7a88bddb0874",
})
EXPECTED["stage_manifests"].append({
    "name": "quickjs-runtime-compile-v1",
    "file": "quickjs-runtime-compile-v1.sha256",
    "sha256": "15614690c79ade16025fded42bc9905fb9aef1add904c6280ef6538f67979538",
    "count": 185,
    "extra_paths": QUICKJS_RUNTIME_EXTRA_PATHS,
    "path_order": "lexical",
})
EXPECTED["quickjs_runtime_compile_base"] = {
    "file": "quickjs-runtime-compile-v1-base.sha256",
    "sha256": "2bd5d6ebe24b70d971ba1e9fbaee069562c333a075f48ff3097d11cf541127a2",
    "count": 185,
    "path_order": "lexical",
    "paired_after": "std-compile-v1",
    "extra_owner": {
        "path": QUICKJS_WASM_PATH,
        "sha256": QUICKJS_WASM_BASE_SHA256,
        "git_blob_sha1": QUICKJS_WASM_BASE_GIT_BLOB_SHA1,
        "final_sha256": QUICKJS_WASM_FINAL_SHA256,
    },
}
EXPECTED["quickjs_runtime_source_paths"] = QUICKJS_RUNTIME_SOURCE_PATHS
EXPECTED["net_owners"] = {
    "file": "net-owners.txt",
    "sha256": "50d21ac22ff6888bd58eee813964c5a843e4a476620e480482cb7a88bddb0874",
    "count": 61,
    "modified": 56,
    "new": 5,
}
EXPECTED["patch_order"].append("quickjs-runtime-compile-v1")
EXPECTED["test_filters"].extend([
    {"package": "thaw-runtime", "filter": "template_strings_reset_tests::raw_reads_reuse_the_registered_handle_and_exact_string_bytes"},
    {"package": "thaw-runtime", "filter": "template_strings_reset_tests::cooked_root_retains_raw_until_the_cooked_handle_expires"},
    {"package": "thaw-runtime", "filter": "radix_string_tests::shortest_radix_spelling_roundtrips_across_every_base"},
    {"package": "thaw-runtime", "filter": "reset_tests::match_metadata_expires_with_its_arena_handle"},
    {"package": "thaw-quickjs", "filter": "tests::webassembly_callback_exception_identity"},
    {"package": "thaw-quickjs", "filter": "tests::webassembly_start_trap_error_kind"},
])
EXPECTED["expected_full_test_names"].update({
    name: [name] for name in (
        "template_strings_reset_tests::raw_reads_reuse_the_registered_handle_and_exact_string_bytes",
        "template_strings_reset_tests::cooked_root_retains_raw_until_the_cooked_handle_expires",
        "radix_string_tests::shortest_radix_spelling_roundtrips_across_every_base",
        "reset_tests::match_metadata_expires_with_its_arena_handle",
        "tests::webassembly_callback_exception_identity",
        "tests::webassembly_start_trap_error_kind",
    )
})

EXPECTED["payload_patches"]["quickjs-api-followup-v1.patch"] = "3e4daa17ed3234d378d02ce34b19178a0bb3a6fae744d2a134b84c286d9fb1d0"
EXPECTED["artifacts"].update({
    "quickjs-api-followup-v1-base.sha256": "daf200a9aea71a2726db0c2754dbeb9da958bfafb53792a4d13adada892e4ae0",
    "quickjs-api-followup-v1.sha256": "2fc7c97c06059974628930e661751c3a4cf7603bb1c2e795fe6a56d7f862d06b",
    "net-owners.txt": "6c5e242bcbe3a8e247943cc644174df43d08ad9af376422d0ffed91bdb0b4ba0",
})
EXPECTED["stage_manifests"].append({
    "name": "quickjs-api-followup-v1",
    "file": "quickjs-api-followup-v1.sha256",
    "sha256": "2fc7c97c06059974628930e661751c3a4cf7603bb1c2e795fe6a56d7f862d06b",
    "count": 186,
    "extra_paths": QUICKJS_API_FOLLOWUP_EXTRA_PATHS,
    "path_order": "lexical",
})
EXPECTED["quickjs_api_followup_base"] = {
    "file": "quickjs-api-followup-v1-base.sha256",
    "sha256": "daf200a9aea71a2726db0c2754dbeb9da958bfafb53792a4d13adada892e4ae0",
    "count": 186,
    "path_order": "lexical",
    "paired_after": "quickjs-runtime-compile-v1",
    "extra_owner": {
        "path": QUICKJS_TESTS_PATH,
        "sha256": QUICKJS_TESTS_BASE_SHA256,
        "git_blob_sha1": QUICKJS_TESTS_BASE_GIT_BLOB_SHA1,
        "final_sha256": QUICKJS_TESTS_FINAL_SHA256,
    },
}
EXPECTED["quickjs_api_followup_source_paths"] = QUICKJS_API_FOLLOWUP_SOURCE_PATHS
EXPECTED["net_owners"] = {
    "file": "net-owners.txt",
    "sha256": "6c5e242bcbe3a8e247943cc644174df43d08ad9af376422d0ffed91bdb0b4ba0",
    "count": 62,
    "modified": 57,
    "new": 5,
}
EXPECTED["patch_order"].append("quickjs-api-followup-v1")
EXPECTED["test_filters"].extend([
    {"package": "thaw-quickjs", "filter": "tests::terminal_pending_work_is_separate_from_failure_and_exit_code"},
    {"package": "thaw-quickjs", "filter": "tests::private_graph_arguments_do_not_revive_user_marker_shapes"},
])
EXPECTED["expected_full_test_names"].update({
    name: [name] for name in (
        "tests::terminal_pending_work_is_separate_from_failure_and_exit_code",
        "tests::private_graph_arguments_do_not_revive_user_marker_shapes",
    )
})


class VerificationError(RuntimeError):
    """A source identity, input pin, or reconstruction contract failed."""


class EvidenceError(VerificationError):
    """Required validation evidence could not be persisted."""


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _safe_rel(value: str, label: str) -> PurePosixPath:
    if not isinstance(value, str) or not value or "\\" in value:
        raise VerificationError(f"{label}: unsafe path {value!r}")
    path = PurePosixPath(value)
    if path.is_absolute() or any(part in ("", ".", "..") for part in path.parts):
        raise VerificationError(f"{label}: absolute or traversal path {value!r}")
    if path.as_posix() != value:
        raise VerificationError(f"{label}: non-canonical path {value!r}")
    return path


def _path_without_symlinks(root: Path, rel: str, label: str) -> Path:
    pure = _safe_rel(rel, label)
    root = root.resolve()
    current = root
    for part in pure.parts:
        current = current / part
        try:
            if current.is_symlink():
                raise VerificationError(f"{label}: symlink path is not allowed: {rel}")
        except OSError as exc:
            raise VerificationError(f"{label}: cannot inspect {rel}: {exc}") from exc
    resolved = current.resolve(strict=False)
    if resolved != root and root not in resolved.parents:
        raise VerificationError(f"{label}: path escapes root: {rel}")
    return current


def _tracked_path(root: Path, rel: str) -> Path:
    """Resolve a Git path without following parent symlinks; the leaf may be a link."""
    pure = _safe_rel(rel, "tracked path")
    current = root.resolve()
    for part in pure.parts[:-1]:
        current = current / part
        if current.is_symlink():
            raise VerificationError(f"tracked path has a symlink parent: {rel}")
    return current / pure.parts[-1]


def _json_no_duplicates(data: bytes, label: str) -> Any:
    def hook(pairs):
        out = {}
        for key, value in pairs:
            if key in out:
                raise VerificationError(f"{label}: duplicate JSON key {key!r}")
            out[key] = value
        return out

    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=hook)
    except VerificationError:
        raise
    except Exception as exc:
        raise VerificationError(f"{label}: invalid JSON: {exc}") from exc


def _run_git(root: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise VerificationError(f"git {' '.join(args)} failed ({result.returncode}): {result.stderr.strip()}")
    return result.stdout.strip()


def _git_tree_entries(root: Path) -> dict[str, tuple[str, str, str]]:
    result = subprocess.run(
        ["git", "-C", str(root), "ls-tree", "-r", "-z", "--full-tree", "HEAD"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        message = result.stderr.decode("utf-8", errors="replace").strip()
        raise VerificationError(f"git ls-tree failed ({result.returncode}): {message}")
    entries: dict[str, tuple[str, str, str]] = {}
    for record in result.stdout.split(b"\0"):
        if not record:
            continue
        try:
            metadata, raw_path = record.split(b"\t", 1)
            mode, object_type, object_id = metadata.decode("ascii").split(" ")
            rel = os.fsdecode(raw_path)
        except Exception as exc:
            raise VerificationError(f"git ls-tree returned malformed tracked entry: {record[:120]!r}") from exc
        _safe_rel(rel, "tracked baseline path")
        if mode == "160000":
            raise VerificationError(f"baseline contains an unsupported unmaterialized gitlink: {rel}")
        if mode not in ("100644", "100755", "120000") or object_type != "blob":
            raise VerificationError(f"baseline has unsupported tracked type/mode {object_type}/{mode}: {rel}")
        if rel in entries:
            raise VerificationError(f"git tree contains duplicate tracked path: {rel}")
        entries[rel] = (mode, object_type, object_id)
    if not entries:
        raise VerificationError("baseline Git tree is empty")
    return entries


def _git_blob_sha1(data: bytes) -> str:
    header = f"blob {len(data)}\0".encode("ascii")
    return hashlib.sha1(header + data).hexdigest()


def _verify_worktree_matches_tree(root: Path) -> None:
    tracked = _git_tree_entries(root)
    actual = _inventory(root)
    tracked_paths = set(tracked)
    actual_paths = set(actual)
    if tracked_paths != actual_paths:
        extra = sorted(actual_paths - tracked_paths)
        missing = sorted(tracked_paths - actual_paths)
        if extra:
            raise VerificationError(f"baseline worktree has extra file outside Git tree: {extra[0]}")
        raise VerificationError(f"baseline worktree is missing tracked Git tree path: {missing[0]}")
    for rel, (mode, _object_type, object_id) in tracked.items():
        path = _tracked_path(root, rel)
        actual_type, actual_mode, _sha256 = actual[rel]
        if mode == "120000":
            if actual_type != "symlink":
                raise VerificationError(f"tracked Git tree symlink type mismatch: {rel}")
            data = os.fsencode(os.readlink(path))
        else:
            if actual_type != "file":
                raise VerificationError(f"tracked Git tree regular-file type mismatch: {rel}")
            expected_mode = 0o755 if mode == "100755" else 0o644
            if actual_mode != expected_mode:
                raise VerificationError(f"tracked Git tree file mode mismatch for {rel}: expected {expected_mode:04o}, got {actual_mode:04o}")
            data = path.read_bytes()
        if _git_blob_sha1(data) != object_id:
            raise VerificationError(f"tracked Git tree blob mismatch for {rel}")


def _baseline_identity(baseline: Path) -> dict[str, str]:
    if not baseline.is_dir() or baseline.is_symlink():
        raise VerificationError(f"baseline directory is missing or symlinked: {baseline}")
    head = _run_git(baseline, "rev-parse", "HEAD")
    tree = _run_git(baseline, "rev-parse", "HEAD^{tree}")
    dirty = _run_git(baseline, "status", "--porcelain=v1", "--untracked-files=all")
    if dirty:
        raise VerificationError(f"baseline must be clean; dirty status: {dirty[:600]}")
    expected = EXPECTED["baseline"]
    if head != expected["commit"]:
        raise VerificationError(f"baseline commit mismatch: expected {expected['commit']}, got {head}")
    if tree != expected["tree"]:
        raise VerificationError(f"baseline tree mismatch: expected {expected['tree']}, got {tree}")
    _verify_worktree_matches_tree(baseline)
    return {"commit": head, "tree": tree}


def _paths_overlap(first: Path, second: Path) -> bool:
    a = first.resolve(strict=False)
    b = second.resolve(strict=False)
    return a == b or a in b.parents or b in a.parents


def _validate_independent_paths(baseline: Path, candidate: Path, payload: Path, evidence: Path | None = None) -> None:
    roots = [("baseline", baseline), ("candidate", candidate), ("payload", payload)]
    if evidence is not None:
        if evidence.exists() and not evidence.is_dir():
            raise VerificationError(f"evidence path must be a directory or absent, not a file: {evidence}")
        roots.append(("evidence", evidence))
    for index, (name, path) in enumerate(roots):
        if path.is_symlink():
            raise VerificationError(f"{name} root must not be a symlink: {path}")
        for other_name, other_path in roots[index + 1 :]:
            if _paths_overlap(path, other_path):
                raise VerificationError(f"{name} and {other_name} roots must be independent and disjoint")


def _parse_sha_manifest(path: Path, label: str, path_order: str = "components") -> list[tuple[str, str]]:
    if not path.is_file() or path.is_symlink():
        raise VerificationError(f"{label}: manifest is missing or symlinked: {path}")
    result: list[tuple[str, str]] = []
    seen: set[str] = set()
    previous: Any = None
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except Exception as exc:
        raise VerificationError(f"{label}: cannot read manifest: {exc}") from exc
    for number, line in enumerate(lines, 1):
        match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if not match:
            raise VerificationError(f"{label}: malformed sha256 line {number}")
        digest, rel = match.groups()
        _safe_rel(rel, f"{label} line {number}")
        if rel in seen:
            raise VerificationError(f"{label}: duplicate path {rel}")
        if path_order == "components":
            order_key: Any = tuple(PurePosixPath(rel).parts)
        elif path_order == "lexical":
            order_key = rel
        else:
            raise VerificationError(f"{label}: unknown path ordering {path_order!r}")
        if previous is not None and order_key <= previous:
            raise VerificationError(f"{label}: paths are not strictly sorted at {rel}")
        seen.add(rel)
        previous = order_key
        result.append((digest, rel))
    if not result:
        raise VerificationError(f"{label}: empty manifest")
    return result


def _is_subset_path(rel: str, roots: list[str] | None = None) -> bool:
    roots = roots or EXPECTED["subset_roots"]
    return any(rel == root or rel.startswith(root.rstrip("/") + "/") for root in roots)


def _subset_inventory(tree: Path) -> set[str]:
    found: set[str] = set()
    for rel_root in EXPECTED["subset_roots"]:
        root_path = _path_without_symlinks(tree, rel_root, "subset root")
        if not root_path.exists():
            continue
        if root_path.is_file():
            found.add(rel_root)
            continue
        for current, dirs, files in os.walk(root_path, followlinks=False):
            current_path = Path(current)
            for name in list(dirs):
                entry = current_path / name
                if entry.is_symlink():
                    rel = entry.relative_to(tree).as_posix()
                    raise VerificationError(f"native subset contains symlink directory: {rel}")
            for name in files:
                entry = current_path / name
                rel = entry.relative_to(tree).as_posix()
                if entry.is_symlink():
                    raise VerificationError(f"native subset contains symlink file: {rel}")
                if not entry.is_file():
                    raise VerificationError(f"native subset contains non-regular file: {rel}")
                found.add(rel)
    return found


def verify_stage(
    tree: Path,
    manifest: Path,
    expected_count: int,
    extra_paths: list[str] | None = None,
    path_order: str = "components",
) -> dict[str, Any]:
    """Validate one source subset against a canonical sorted sha256 manifest."""
    extra_paths = extra_paths or []
    allowed_extra_sets = [
        [EXPECTED.get("repair_base", {}).get("extra_owner", {}).get("path")],
        FORCED_ROOT_STAGE_EXTRA_PATHS,
        STD_COMPILE_EXTRA_PATHS,
        QUICKJS_RUNTIME_EXTRA_PATHS,
        QUICKJS_API_FOLLOWUP_EXTRA_PATHS,
    ]
    if extra_paths and extra_paths not in allowed_extra_sets:
        raise VerificationError(f"stage has an unapproved extra path scope: {extra_paths!r}")
    rows = _parse_sha_manifest(manifest, "stage manifest", path_order=path_order)
    if len(rows) != expected_count:
        raise VerificationError(f"stage manifest count mismatch: expected {expected_count}, got {len(rows)}")
    if not tree.is_dir() or tree.is_symlink():
        raise VerificationError(f"stage tree is missing or symlinked: {tree}")
    root = tree.resolve()
    expected_paths = {rel for _, rel in rows}
    bad_paths = sorted(rel for rel in expected_paths if not _is_subset_path(rel) and rel not in extra_paths)
    if bad_paths:
        raise VerificationError(f"stage manifest contains path outside native subset: {bad_paths[0]}")
    for digest, rel in rows:
        file_path = _path_without_symlinks(root, rel, "stage manifest")
        if not file_path.exists():
            raise VerificationError(f"stage file missing (including possible control owner): {rel}")
        if not file_path.is_file():
            raise VerificationError(f"stage path is not a regular file: {rel}")
        actual = _sha(file_path.read_bytes())
        if actual != digest:
            raise VerificationError(f"stage hash mismatch for {rel}: expected {digest}, got {actual}")
    actual_paths = _subset_inventory(root)
    for rel in extra_paths:
        extra_file = _path_without_symlinks(root, rel, "stage-scoped extra owner")
        if not extra_file.is_file():
            raise VerificationError(f"stage-scoped extra owner is missing or not a regular file: {rel}")
        actual_paths.add(rel)
    if actual_paths != expected_paths:
        missing = sorted(expected_paths - actual_paths)
        extra = sorted(actual_paths - expected_paths)
        if missing:
            raise VerificationError(f"native subset inventory is missing {missing[0]}")
        raise VerificationError(f"native subset inventory has extra file {extra[0]}")
    return {"count": len(rows), "manifest_sha256": _sha(manifest.read_bytes()), "files": sorted(expected_paths)}


def _read_owner_map(payload: Path) -> dict[str, str]:
    descriptor = EXPECTED["native_base"]
    path = _path_without_symlinks(payload, descriptor["file"], "native-base manifest")
    rows = _parse_sha_manifest(path, "native-base manifest", path_order="lexical")
    if len(rows) != descriptor["count"]:
        raise VerificationError(f"native-base owner count mismatch: expected {descriptor['count']}, got {len(rows)}")
    return {rel: digest for digest, rel in rows}


def _read_net_roster(payload: Path) -> dict[str, str]:
    descriptor = EXPECTED["net_owners"]
    path = _path_without_symlinks(payload, descriptor["file"], "net-owner roster")
    try:
        rows = path.read_text(encoding="utf-8").splitlines()
    except Exception as exc:
        raise VerificationError(f"net-owner roster unreadable: {exc}") from exc
    roster: dict[str, str] = {}
    previous = ""
    for number, line in enumerate(rows, 1):
        match = re.fullmatch(r"(modified|new|added)  (.+)", line)
        if not match:
            raise VerificationError(f"net-owner roster malformed at line {number}")
        source_status, rel = match.groups()
        # The frozen source roster spells inherited new-file rows "added";
        # normalize that one allowed spelling to the verifier's internal "new".
        status = "new" if source_status == "added" else source_status
        _safe_rel(rel, f"net-owner line {number}")
        if rel in roster:
            raise VerificationError(f"net-owner roster has duplicate path {rel}")
        if previous and rel <= previous:
            raise VerificationError(f"net-owner roster is not sorted by path at {rel}")
        previous = rel
        roster[rel] = status
    counts = {status: sum(value == status for value in roster.values()) for status in ("modified", "new")}
    if len(roster) != descriptor["count"] or counts["modified"] != descriptor["modified"] or counts["new"] != descriptor["new"]:
        raise VerificationError(
            "net-owner roster count mismatch: "
            f"expected {descriptor['count']} ({descriptor['modified']} modified, {descriptor['new']} new), "
            f"got {len(roster)} ({counts['modified']} modified, {counts['new']} new)"
        )
    return roster


def _verify_dependency_compile_inputs(
    payload: Path,
    stage_by_name: dict[str, dict[str, Any]],
    net: dict[str, str],
) -> dict[str, Any]:
    """Pin the ninth patch to the immediately preceding 183-file wire final."""
    base = EXPECTED.get("dependency_compile_base")
    previous = stage_by_name.get("forced-root-wire-v1")
    stage = stage_by_name.get("dependency-compile-v1")
    if not isinstance(base, dict) or previous is None or stage is None:
        raise VerificationError("dependency compile paired base or stage is missing")
    if base.get("paired_after") != previous["name"]:
        raise VerificationError("dependency compile paired base must name the forced-root wire predecessor")
    patch_index = EXPECTED["patch_order"].index("dependency-compile-v1")
    if patch_index == 0 or EXPECTED["patch_order"][patch_index - 1] != base["paired_after"]:
        raise VerificationError("dependency compile paired base does not immediately precede its patch stage")
    if base.get("path_order") != "lexical" or previous.get("path_order") != "lexical":
        raise VerificationError("dependency compile base and predecessor must retain lexical path ordering")
    if stage.get("path_order") != "lexical" or stage.get("extra_paths") != FORCED_ROOT_STAGE_EXTRA_PATHS:
        raise VerificationError("dependency compile final must retain the three existing stage-scoped extra paths")
    if previous.get("extra_paths") != FORCED_ROOT_STAGE_EXTRA_PATHS:
        raise VerificationError("dependency compile predecessor no longer carries the pinned three extra paths")
    if base.get("count") != previous.get("count") or stage.get("count") != base.get("count"):
        raise VerificationError("dependency compile base and final counts must equal the preceding 183-file stage")
    if base.get("file") != "dependency-compile-v1-base.sha256" or stage.get("file") != "dependency-compile-v1.sha256":
        raise VerificationError("dependency compile paired input filenames differ from the pinned contract")
    for descriptor in (base, stage):
        if descriptor.get("sha256") != EXPECTED["artifacts"].get(descriptor["file"]):
            raise VerificationError(f"dependency compile manifest pin is stale for {descriptor['file']}")
    base_path = _path_without_symlinks(payload, base["file"], "dependency compile paired base")
    previous_path = _path_without_symlinks(payload, previous["file"], "dependency compile predecessor final")
    final_path = _path_without_symlinks(payload, stage["file"], "dependency compile final")
    base_bytes = base_path.read_bytes()
    previous_bytes = previous_path.read_bytes()
    final_bytes = final_path.read_bytes()
    if _sha(base_bytes) != base["sha256"] or _sha(final_bytes) != stage["sha256"]:
        raise VerificationError("dependency compile paired base or final manifest hash differs from its descriptor")
    if base_bytes != previous_bytes:
        raise VerificationError("dependency compile paired base is not byte-identical to the immediately preceding wire final")
    base_rows = _parse_sha_manifest(base_path, "dependency compile paired base", path_order="lexical")
    previous_rows = _parse_sha_manifest(previous_path, "dependency compile predecessor final", path_order="lexical")
    final_rows = _parse_sha_manifest(final_path, "dependency compile final", path_order="lexical")
    base_map = {rel: digest for digest, rel in base_rows}
    previous_map = {rel: digest for digest, rel in previous_rows}
    final_map = {rel: digest for digest, rel in final_rows}
    expected_count = base["count"]
    if len(base_rows) != expected_count or len(previous_rows) != expected_count or base_map != previous_map:
        raise VerificationError("dependency compile base differs from the exact 183-file wire final")
    if len(final_rows) != expected_count or set(final_map) != set(base_map):
        raise VerificationError("dependency compile final changed the predecessor's exact 183-path set")
    changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
    if len(DEPENDENCY_SOURCE_PATHS) != 4 or changed != set(DEPENDENCY_SOURCE_PATHS):
        raise VerificationError("dependency compile final must modify only its four pinned source owners")
    if any(net.get(rel) != "modified" for rel in DEPENDENCY_SOURCE_PATHS):
        raise VerificationError("dependency compile source owners must all be modified net owners")
    return {
        "base_count": len(base_map),
        "final_count": len(final_map),
        "changed_source_owners": sorted(changed),
        "unchanged_source_count": expected_count - len(changed),
        "paired_after": previous["name"],
    }


def _verify_std_compile_inputs(
    payload: Path,
    baseline: Path,
    stage_by_name: dict[str, dict[str, Any]],
    net: dict[str, str],
) -> dict[str, Any]:
    """Pin the tenth patch to the ninth final plus its exact baseline HTTP owner."""
    base = EXPECTED.get("std_compile_base")
    previous = stage_by_name.get("dependency-compile-v1")
    stage = stage_by_name.get("std-compile-v1")
    if not isinstance(base, dict) or previous is None or stage is None:
        raise VerificationError("std compile paired base or stage is missing")
    if base.get("paired_after") != previous["name"]:
        raise VerificationError("std compile paired base must name the dependency compile predecessor")
    patch_index = EXPECTED["patch_order"].index("std-compile-v1")
    if patch_index == 0 or EXPECTED["patch_order"][patch_index - 1] != base["paired_after"]:
        raise VerificationError("std compile paired base does not immediately precede its patch stage")
    if base.get("path_order") != "lexical" or previous.get("path_order") != "lexical":
        raise VerificationError("std compile base and predecessor must retain lexical path ordering")
    if previous.get("extra_paths") != FORCED_ROOT_STAGE_EXTRA_PATHS:
        raise VerificationError("std compile predecessor no longer carries the pinned three extra paths")
    if stage.get("path_order") != "lexical" or stage.get("extra_paths") != STD_COMPILE_EXTRA_PATHS:
        raise VerificationError("std compile final must retain the exact stage-scoped HTTP path")
    if base.get("count") != previous["count"] + 1 or stage.get("count") != base.get("count"):
        raise VerificationError("std compile paired base and final must contain the exact 184-file scope")
    if base.get("file") != "std-compile-v1-base.sha256" or stage.get("file") != "std-compile-v1.sha256":
        raise VerificationError("std compile paired input filenames differ from the pinned contract")
    for descriptor in (base, stage):
        if descriptor.get("sha256") != EXPECTED["artifacts"].get(descriptor["file"]):
            raise VerificationError(f"std compile manifest pin is stale for {descriptor['file']}")

    owner = base.get("extra_owner")
    expected_owner = {
        "path": STD_HTTP_PATH,
        "sha256": STD_HTTP_BASE_SHA256,
        "git_blob_sha1": STD_HTTP_BASE_GIT_BLOB_SHA1,
        "final_sha256": STD_HTTP_FINAL_SHA256,
    }
    if owner != expected_owner:
        raise VerificationError("std compile paired base HTTP source identity differs from the pinned baseline")
    http_path = _path_without_symlinks(baseline, STD_HTTP_PATH, "std compile baseline HTTP owner")
    if not http_path.is_file() or http_path.is_symlink():
        raise VerificationError(f"std compile baseline HTTP owner is missing or not a regular file: {STD_HTTP_PATH}")
    http_bytes = http_path.read_bytes()
    if _sha(http_bytes) != STD_HTTP_BASE_SHA256 or _git_blob_sha1(http_bytes) != STD_HTTP_BASE_GIT_BLOB_SHA1:
        raise VerificationError("std compile baseline HTTP owner SHA-256 or Git blob identity mismatch")

    base_path = _path_without_symlinks(payload, base["file"], "std compile paired base")
    previous_path = _path_without_symlinks(payload, previous["file"], "std compile predecessor final")
    final_path = _path_without_symlinks(payload, stage["file"], "std compile final")
    base_bytes, previous_bytes, final_bytes = base_path.read_bytes(), previous_path.read_bytes(), final_path.read_bytes()
    if (
        _sha(base_bytes) != base["sha256"]
        or _sha(previous_bytes) != previous["sha256"]
        or _sha(final_bytes) != stage["sha256"]
        or previous["sha256"] != EXPECTED["artifacts"].get(previous["file"])
    ):
        raise VerificationError("std compile paired base or final manifest hash differs from its descriptor")
    previous_rows = _parse_sha_manifest(previous_path, "std compile predecessor final", path_order="lexical")
    base_rows = _parse_sha_manifest(base_path, "std compile paired base", path_order="lexical")
    final_rows = _parse_sha_manifest(final_path, "std compile final", path_order="lexical")
    previous_map = {rel: digest for digest, rel in previous_rows}
    base_map = {rel: digest for digest, rel in base_rows}
    final_map = {rel: digest for digest, rel in final_rows}
    expected_base = dict(previous_map)
    if STD_HTTP_PATH in expected_base:
        raise VerificationError("std compile HTTP baseline owner already exists in its predecessor stage")
    expected_base[STD_HTTP_PATH] = STD_HTTP_BASE_SHA256
    if len(previous_rows) != previous["count"] or len(base_rows) != base["count"] or base_map != expected_base:
        raise VerificationError("std compile paired base differs from the exact dependency final plus baseline HTTP owner")
    if len(final_rows) != stage["count"] or set(final_map) != set(base_map):
        raise VerificationError("std compile final changed the paired base's exact 184-path set")
    changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
    if changed != set(STD_COMPILE_SOURCE_PATHS) or final_map.get(STD_HTTP_PATH) != STD_HTTP_FINAL_SHA256:
        raise VerificationError("std compile final must modify only the pinned HTTP and JSON owners")
    if any(net.get(rel) != "modified" for rel in STD_COMPILE_SOURCE_PATHS):
        raise VerificationError("std compile HTTP and JSON source owners must be modified net owners")
    return {
        "base_count": len(base_map),
        "final_count": len(final_map),
        "changed_source_owners": sorted(changed),
        "unchanged_source_count": len(base_map) - len(changed),
        "paired_after": previous["name"],
        "http_baseline_sha256": STD_HTTP_BASE_SHA256,
        "http_baseline_git_blob_sha1": STD_HTTP_BASE_GIT_BLOB_SHA1,
    }


def _verify_quickjs_runtime_inputs(
    payload: Path, baseline: Path, stage_by_name: dict[str, dict[str, Any]], net: dict[str, str]
) -> dict[str, Any]:
    """Pin stage eleven to stage ten plus original baseline wasm.rs."""
    base = EXPECTED.get("quickjs_runtime_compile_base")
    previous = stage_by_name.get("std-compile-v1")
    stage = stage_by_name.get("quickjs-runtime-compile-v1")
    if not isinstance(base, dict) or previous is None or stage is None:
        raise VerificationError("QuickJS/runtime paired base or stage is missing")
    quickjs_runtime_index = EXPECTED["patch_order"].index("quickjs-runtime-compile-v1")
    if (
        base.get("paired_after") != previous["name"]
        or quickjs_runtime_index == 0
        or EXPECTED["patch_order"][quickjs_runtime_index - 1] != "std-compile-v1"
        or EXPECTED["patch_order"][quickjs_runtime_index + 1] != "quickjs-api-followup-v1"
    ):
        raise VerificationError("QuickJS/runtime paired base must immediately follow std-compile-v1")
    if base.get("path_order") != "lexical" or previous.get("path_order") != "lexical":
        raise VerificationError("QuickJS/runtime base and predecessor must retain lexical path ordering")
    if previous.get("extra_paths") != STD_COMPILE_EXTRA_PATHS or stage.get("extra_paths") != QUICKJS_RUNTIME_EXTRA_PATHS:
        raise VerificationError("QuickJS/runtime stage has an unexpected stage-scoped path set")
    if stage.get("path_order") != "lexical" or base.get("count") != previous["count"] + 1 or stage.get("count") != base["count"]:
        raise VerificationError("QuickJS/runtime paired base and final must contain exactly 185 files")
    if base.get("file") != "quickjs-runtime-compile-v1-base.sha256" or stage.get("file") != "quickjs-runtime-compile-v1.sha256":
        raise VerificationError("QuickJS/runtime paired input filenames differ from the pinned contract")
    for descriptor in (base, stage):
        if descriptor.get("sha256") != EXPECTED["artifacts"].get(descriptor["file"]):
            raise VerificationError(f"QuickJS/runtime manifest pin is stale for {descriptor['file']}")
    expected_owner = {
        "path": QUICKJS_WASM_PATH,
        "sha256": QUICKJS_WASM_BASE_SHA256,
        "git_blob_sha1": QUICKJS_WASM_BASE_GIT_BLOB_SHA1,
        "final_sha256": QUICKJS_WASM_FINAL_SHA256,
    }
    if base.get("extra_owner") != expected_owner:
        raise VerificationError("QuickJS/runtime paired base wasm.rs identity differs from the pinned baseline")
    wasm_path = _path_without_symlinks(baseline, QUICKJS_WASM_PATH, "QuickJS/runtime baseline wasm owner")
    if not wasm_path.is_file() or wasm_path.is_symlink():
        raise VerificationError(f"QuickJS/runtime baseline wasm owner is missing or not a regular file: {QUICKJS_WASM_PATH}")
    wasm_bytes = wasm_path.read_bytes()
    if _sha(wasm_bytes) != QUICKJS_WASM_BASE_SHA256 or _git_blob_sha1(wasm_bytes) != QUICKJS_WASM_BASE_GIT_BLOB_SHA1:
        raise VerificationError("baseline QuickJS wasm owner SHA-256 or Git blob identity mismatch")
    base_path = _path_without_symlinks(payload, base["file"], "QuickJS/runtime paired base")
    previous_path = _path_without_symlinks(payload, previous["file"], "QuickJS/runtime predecessor final")
    final_path = _path_without_symlinks(payload, stage["file"], "QuickJS/runtime final")
    previous_rows = _parse_sha_manifest(previous_path, "QuickJS/runtime predecessor final", path_order="lexical")
    base_rows = _parse_sha_manifest(base_path, "QuickJS/runtime paired base", path_order="lexical")
    final_rows = _parse_sha_manifest(final_path, "QuickJS/runtime final", path_order="lexical")
    previous_map = {rel: digest for digest, rel in previous_rows}
    base_map = {rel: digest for digest, rel in base_rows}
    final_map = {rel: digest for digest, rel in final_rows}
    expected_base = dict(previous_map)
    if QUICKJS_WASM_PATH in expected_base:
        raise VerificationError("QuickJS/runtime paired base wasm.rs is already in the preceding source set")
    expected_base[QUICKJS_WASM_PATH] = QUICKJS_WASM_BASE_SHA256
    if base_map != expected_base or len(base_rows) != base["count"]:
        raise VerificationError("QuickJS/runtime paired base differs from std final plus original baseline wasm.rs")
    if set(final_map) != set(base_map) or len(final_rows) != stage["count"]:
        raise VerificationError("QuickJS/runtime final changed the paired base path set")
    changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
    if changed != set(QUICKJS_RUNTIME_SOURCE_PATHS) or final_map.get(QUICKJS_WASM_PATH) != QUICKJS_WASM_FINAL_SHA256:
        raise VerificationError("QuickJS/runtime final must change exactly the five pinned source owners")
    if any(net.get(rel) != "modified" for rel in QUICKJS_RUNTIME_SOURCE_PATHS):
        raise VerificationError("QuickJS/runtime source owners must all be modified net owners")
    return {
        "base_count": len(base_map),
        "final_count": len(final_map),
        "changed_source_owners": sorted(changed),
        "unchanged_source_count": len(base_map) - len(changed),
        "paired_after": previous["name"],
    }


def _verify_quickjs_api_followup_inputs(
    payload: Path, baseline: Path, stage_by_name: dict[str, dict[str, Any]], net: dict[str, str]
) -> dict[str, Any]:
    """Pin stage twelve to stage eleven plus the original QuickJS tests.rs."""
    base = EXPECTED.get("quickjs_api_followup_base")
    previous = stage_by_name.get("quickjs-runtime-compile-v1")
    stage = stage_by_name.get("quickjs-api-followup-v1")
    if not isinstance(base, dict) or previous is None or stage is None:
        raise VerificationError("QuickJS API follow-up paired base or stage is missing")
    if base.get("paired_after") != previous["name"] or EXPECTED["patch_order"][-2:] != ["quickjs-runtime-compile-v1", "quickjs-api-followup-v1"]:
        raise VerificationError("QuickJS API follow-up paired base must immediately follow quickjs-runtime-compile-v1")
    if base.get("path_order") != "lexical" or previous.get("path_order") != "lexical":
        raise VerificationError("QuickJS API follow-up base and predecessor must retain lexical path ordering")
    if previous.get("extra_paths") != QUICKJS_RUNTIME_EXTRA_PATHS or stage.get("extra_paths") != QUICKJS_API_FOLLOWUP_EXTRA_PATHS:
        raise VerificationError("QuickJS API follow-up stage has an unexpected stage-scoped path set")
    if stage.get("path_order") != "lexical" or base.get("count") != previous["count"] + 1 or stage.get("count") != base["count"]:
        raise VerificationError("QuickJS API follow-up paired base and final must contain exactly the predecessor plus baseline tests.rs")
    if base.get("file") != "quickjs-api-followup-v1-base.sha256" or stage.get("file") != "quickjs-api-followup-v1.sha256":
        raise VerificationError("QuickJS API follow-up paired input filenames differ from the pinned contract")
    for descriptor in (base, stage):
        if descriptor.get("sha256") != EXPECTED["artifacts"].get(descriptor["file"]):
            raise VerificationError(f"QuickJS API follow-up manifest pin is stale for {descriptor['file']}")
    expected_owner = {
        "path": QUICKJS_TESTS_PATH,
        "sha256": QUICKJS_TESTS_BASE_SHA256,
        "git_blob_sha1": QUICKJS_TESTS_BASE_GIT_BLOB_SHA1,
        "final_sha256": QUICKJS_TESTS_FINAL_SHA256,
    }
    if base.get("extra_owner") != expected_owner:
        raise VerificationError("QuickJS API follow-up paired base tests.rs identity differs from the pinned baseline")
    tests_path = _path_without_symlinks(baseline, QUICKJS_TESTS_PATH, "QuickJS API follow-up baseline tests owner")
    if not tests_path.is_file() or tests_path.is_symlink():
        raise VerificationError(f"QuickJS API follow-up baseline tests owner is missing or not a regular file: {QUICKJS_TESTS_PATH}")
    tests_bytes = tests_path.read_bytes()
    if _sha(tests_bytes) != QUICKJS_TESTS_BASE_SHA256 or _git_blob_sha1(tests_bytes) != QUICKJS_TESTS_BASE_GIT_BLOB_SHA1:
        raise VerificationError("baseline QuickJS tests.rs owner SHA-256 or Git blob identity mismatch")
    base_path = _path_without_symlinks(payload, base["file"], "QuickJS API follow-up paired base")
    previous_path = _path_without_symlinks(payload, previous["file"], "QuickJS API follow-up predecessor final")
    final_path = _path_without_symlinks(payload, stage["file"], "QuickJS API follow-up final")
    previous_rows = _parse_sha_manifest(previous_path, "QuickJS API follow-up predecessor final", path_order="lexical")
    base_rows = _parse_sha_manifest(base_path, "QuickJS API follow-up paired base", path_order="lexical")
    final_rows = _parse_sha_manifest(final_path, "QuickJS API follow-up final", path_order="lexical")
    previous_map = {rel: digest for digest, rel in previous_rows}
    base_map = {rel: digest for digest, rel in base_rows}
    final_map = {rel: digest for digest, rel in final_rows}
    expected_base = dict(previous_map)
    if QUICKJS_TESTS_PATH in expected_base:
        raise VerificationError("QuickJS API follow-up paired base tests.rs is already in the preceding source set")
    expected_base[QUICKJS_TESTS_PATH] = QUICKJS_TESTS_BASE_SHA256
    if base_map != expected_base or len(base_rows) != base["count"]:
        raise VerificationError("QuickJS API follow-up paired base differs from QuickJS/runtime final plus original baseline tests.rs")
    if set(final_map) != set(base_map) or len(final_rows) != stage["count"]:
        raise VerificationError("QuickJS API follow-up final changed the paired base path set")
    changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
    if changed != set(QUICKJS_API_FOLLOWUP_SOURCE_PATHS) or final_map.get(QUICKJS_TESTS_PATH) != QUICKJS_TESTS_FINAL_SHA256:
        raise VerificationError("QuickJS API follow-up final must change exactly the three pinned QuickJS source owners")
    if any(net.get(rel) != "modified" for rel in QUICKJS_API_FOLLOWUP_SOURCE_PATHS):
        raise VerificationError("QuickJS API follow-up source owners must all be modified net owners")
    return {
        "base_count": len(base_map),
        "final_count": len(final_map),
        "changed_source_owners": sorted(changed),
        "unchanged_source_count": len(base_map) - len(changed),
        "paired_after": previous["name"],
        "tests_baseline_sha256": QUICKJS_TESTS_BASE_SHA256,
        "tests_baseline_git_blob_sha1": QUICKJS_TESTS_BASE_GIT_BLOB_SHA1,
    }


def verify_inputs(baseline: Path, payload: Path) -> dict[str, Any]:
    """Validate descriptor, immutable bytes, clean baseline and roster preconditions."""
    if not payload.is_dir() or payload.is_symlink():
        raise VerificationError(f"payload directory is missing or symlinked: {payload}")
    if _paths_overlap(baseline, payload):
        raise VerificationError("baseline and payload roots must be independent and disjoint")
    pins_path = _path_without_symlinks(payload, "pins.json", "descriptor")
    descriptor = _json_no_duplicates(pins_path.read_bytes(), "pins.json")
    if descriptor != EXPECTED:
        raise VerificationError("pins.json descriptor differs from the immutable in-code expected descriptor")
    current_ref = os.environ.get("GITHUB_REF")
    expected_ref = f"refs/heads/{EXPECTED['validation_branch']}"
    if current_ref and current_ref != expected_ref:
        raise VerificationError(f"workflow ref mismatch: expected {expected_ref}, got {current_ref}")
    identity = _baseline_identity(baseline)

    # Verify baseline-resident copies (patches 1/2, handoff owner manifest and lockfile).
    source_bytes: dict[str, bytes] = {}
    for name, info in EXPECTED["source_files"].items():
        path = _path_without_symlinks(baseline, info["path"], f"baseline source {name}")
        if not path.is_file():
            raise VerificationError(f"baseline source file missing: {info['path']}")
        data = path.read_bytes()
        actual = _sha(data)
        if actual != info["sha256"]:
            raise VerificationError(f"baseline source hash mismatch for {info['path']}: expected {info['sha256']}, got {actual}")
        source_bytes[name] = data

    # Every published payload artifact is independently fixed in code and in the descriptor.
    for rel, expected_hash in EXPECTED["artifacts"].items():
        path = _path_without_symlinks(payload, rel, "payload artifact")
        if not path.is_file():
            raise VerificationError(f"payload artifact missing: {rel}")
        actual = _sha(path.read_bytes())
        if actual != expected_hash:
            raise VerificationError(f"payload artifact hash mismatch for {rel}: expected {expected_hash}, got {actual}")
    for rel, expected_hash in EXPECTED["payload_patches"].items():
        path = _path_without_symlinks(payload, rel, "payload patch")
        if not path.is_file():
            raise VerificationError(f"payload patch missing: {rel}")
        actual = _sha(path.read_bytes())
        if actual != expected_hash:
            raise VerificationError(f"payload patch hash mismatch for {rel}: expected {expected_hash}, got {actual}")
    expected_next_hir_inputs = {"next-hir-v1.patch", "next-hir-v1-base.sha256", "next-hir-v1.sha256"}
    actual_next_hir_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("next-hir-v")}
    if actual_next_hir_inputs - expected_next_hir_inputs:
        unexpected = sorted(actual_next_hir_inputs - expected_next_hir_inputs)
        raise VerificationError(f"unexpected next HIR input: {unexpected[0]}")
    expected_forced_root_inputs = {"forced-root-wire-v1.patch", "forced-root-wire-v1-base.sha256", "forced-root-wire-v1.sha256"}
    actual_forced_root_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("forced-root-wire-v")}
    if actual_forced_root_inputs - expected_forced_root_inputs:
        unexpected = sorted(actual_forced_root_inputs - expected_forced_root_inputs)
        raise VerificationError(f"unexpected forced-root wire input: {unexpected[0]}")
    expected_dependency_inputs = {"dependency-compile-v1.patch", "dependency-compile-v1-base.sha256", "dependency-compile-v1.sha256"}
    actual_dependency_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("dependency-compile-v")}
    if actual_dependency_inputs - expected_dependency_inputs:
        unexpected = sorted(actual_dependency_inputs - expected_dependency_inputs)
        raise VerificationError(f"unexpected dependency compile input: {unexpected[0]}")
    expected_std_compile_inputs = {"std-compile-v1.patch", "std-compile-v1-base.sha256", "std-compile-v1.sha256"}
    actual_std_compile_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("std-compile-v")}
    if actual_std_compile_inputs - expected_std_compile_inputs:
        unexpected = sorted(actual_std_compile_inputs - expected_std_compile_inputs)
        raise VerificationError(f"unexpected std compile input: {unexpected[0]}")
    expected_quickjs_runtime_inputs = {"quickjs-runtime-compile-v1.patch", "quickjs-runtime-compile-v1-base.sha256", "quickjs-runtime-compile-v1.sha256"}
    actual_quickjs_runtime_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("quickjs-runtime-compile-v")}
    if actual_quickjs_runtime_inputs - expected_quickjs_runtime_inputs:
        unexpected = sorted(actual_quickjs_runtime_inputs - expected_quickjs_runtime_inputs)
        raise VerificationError(f"unexpected QuickJS/runtime compile input: {unexpected[0]}")
    expected_quickjs_api_followup_inputs = {"quickjs-api-followup-v1.patch", "quickjs-api-followup-v1-base.sha256", "quickjs-api-followup-v1.sha256"}
    actual_quickjs_api_followup_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("quickjs-api-followup-v")}
    if actual_quickjs_api_followup_inputs - expected_quickjs_api_followup_inputs:
        unexpected = sorted(actual_quickjs_api_followup_inputs - expected_quickjs_api_followup_inputs)
        raise VerificationError(f"unexpected QuickJS API follow-up input: {unexpected[0]}")

    owner_map = _read_owner_map(payload)
    manifest = _json_no_duplicates(source_bytes["handoff_manifest"], "native handoff manifest")
    try:
        owners = manifest["drafts"]["native"]["owners"]
    except (KeyError, TypeError) as exc:
        raise VerificationError("native handoff manifest has no drafts.native.owners array") from exc
    if not isinstance(owners, list) or len(owners) != EXPECTED["native_base"]["count"]:
        raise VerificationError(f"native handoff owner count mismatch: expected {EXPECTED['native_base']['count']}, got {len(owners) if isinstance(owners, list) else 'non-array'}")
    seen: set[str] = set()
    for owner in owners:
        if not isinstance(owner, dict) or owner.get("base_matches_checkpoint") is not True:
            raise VerificationError("native base owner does not have base_matches_checkpoint=true")
        rel = owner.get("path")
        _safe_rel(rel, "native handoff owner")
        if rel in seen:
            raise VerificationError(f"native handoff manifest has duplicate owner {rel}")
        seen.add(rel)
        if owner.get("base_sha256") != owner_map.get(rel):
            raise VerificationError(f"native-base manifest mismatch for owner {rel}")
        base_file = _path_without_symlinks(baseline, rel, "native base owner")
        if not base_file.is_file() or _sha(base_file.read_bytes()) != owner["base_sha256"]:
            raise VerificationError(f"baseline native owner hash mismatch for {rel}")
    if seen != set(owner_map):
        raise VerificationError("native-base manifest owner set differs from handoff manifest")

    net = _read_net_roster(payload)
    for rel, status in net.items():
        baseline_path = _path_without_symlinks(baseline, rel, "net owner")
        if status == "new" and baseline_path.exists():
            raise VerificationError(f"pre-existing new owner must be absent from baseline: {rel}")
        if status == "modified" and (not baseline_path.is_file() or baseline_path.is_symlink()):
            raise VerificationError(f"modified owner is not a baseline regular file: {rel}")

    # Each manifest file is parsed here, before any patch is applied, so malformed or
    # unsafe input cannot become a Cargo-time surprise.
    lexical_stage_names = {"compile-repairs-v1", "cumulative-hir-v1", "next-hir-v1", "forced-root-wire-v1", "dependency-compile-v1", "std-compile-v1", "quickjs-runtime-compile-v1", "quickjs-api-followup-v1"}
    extra_path = EXPECTED["repair_base"]["extra_owner"]["path"]
    extra_paths_by_stage = {
        "compile-repairs-v1": [extra_path],
        "cumulative-hir-v1": [extra_path],
        "next-hir-v1": [extra_path],
        "forced-root-wire-v1": FORCED_ROOT_STAGE_EXTRA_PATHS,
        "dependency-compile-v1": FORCED_ROOT_STAGE_EXTRA_PATHS,
        "std-compile-v1": STD_COMPILE_EXTRA_PATHS,
        "quickjs-runtime-compile-v1": QUICKJS_RUNTIME_EXTRA_PATHS,
        "quickjs-api-followup-v1": QUICKJS_API_FOLLOWUP_EXTRA_PATHS,
    }
    for stage in EXPECTED["stage_manifests"]:
        extra_paths = stage.get("extra_paths", [])
        expected_extra = extra_paths_by_stage.get(stage["name"], [])
        if extra_paths != expected_extra:
            raise VerificationError(f"{stage['name']} has an unexpected stage-scoped extra path set")
        manifest_path = _path_without_symlinks(payload, stage["file"], "stage manifest")
        path_order = stage.get("path_order", "components")
        expected_path_order = "lexical" if stage["name"] in lexical_stage_names else "components"
        if path_order != expected_path_order:
            raise VerificationError(f"{stage['name']} manifest must use {expected_path_order} path order")
        rows = _parse_sha_manifest(manifest_path, stage["name"], path_order=path_order)
        if len(rows) != stage["count"]:
            raise VerificationError(f"{stage['name']} stage manifest count mismatch: expected {stage['count']}, got {len(rows)}")
        for _, rel in rows:
            if not _is_subset_path(rel) and rel not in extra_paths:
                raise VerificationError(f"{stage['name']} manifest path outside native subset: {rel}")
    expected_order = [stage["name"] for stage in EXPECTED["stage_manifests"]]
    if EXPECTED["patch_order"] != expected_order:
        raise VerificationError(f"patch order mismatch: expected stage order {expected_order}")

    # The repair patch is pinned to exactly the preceding 180-file stage plus
    # the one explicitly scoped thaw-std JSON owner. The repair may modify
    # that owner, so only its pre-patch hash is compared to the baseline pin;
    # the post-patch hash is independently fixed by the final manifest.
    repair = EXPECTED["repair_base"]
    if repair.get("path_order", "components") != "components":
        raise VerificationError("paired repair base must retain component path ordering")
    stage_by_name = {stage["name"]: stage for stage in EXPECTED["stage_manifests"]}
    repair_stage = stage_by_name.get("compile-repairs-v1")
    if repair_stage is None or repair_stage.get("path_order") != "lexical":
        raise VerificationError("compile repair manifest must use its pinned lexical path ordering")
    repair_index = EXPECTED["patch_order"].index("compile-repairs-v1")
    if repair_index == 0:
        raise VerificationError("compile repair has no named predecessor stage")
    repair_predecessor_name = EXPECTED["patch_order"][repair_index - 1]
    repair_predecessor = stage_by_name.get(repair_predecessor_name)
    if repair_predecessor is None:
        raise VerificationError("compile repair predecessor stage is missing")
    base_rows = _parse_sha_manifest(
        _path_without_symlinks(payload, repair["file"], "paired repair base"),
        "paired repair base",
        path_order=repair.get("path_order", "components"),
    )
    if len(base_rows) != repair["count"]:
        raise VerificationError(f"paired repair base count mismatch: expected {repair['count']}, got {len(base_rows)}")
    previous_rows = _parse_sha_manifest(
        payload / repair_predecessor["file"],
        f"pre-repair stage {repair_predecessor_name}",
        path_order=repair_predecessor.get("path_order", "components"),
    )
    expected_base = {rel: digest for digest, rel in previous_rows}
    extra = repair["extra_owner"]
    if extra["path"] in expected_base:
        raise VerificationError("paired repair base extra owner is not the pinned thaw-std JSON source")
    base_json = _path_without_symlinks(baseline, extra["path"], "paired repair base JSON owner")
    if not base_json.is_file() or base_json.is_symlink():
        raise VerificationError(f"paired repair base JSON owner is missing or not a regular file: {extra['path']}")
    json_bytes = base_json.read_bytes()
    if _sha(json_bytes) != extra["sha256"] or _git_blob_sha1(json_bytes) != extra["git_blob_sha1"]:
        raise VerificationError("paired repair base JSON owner does not match its pinned SHA-256 and Git blob SHA-1")
    expected_base[extra["path"]] = extra["sha256"]
    if {rel: digest for digest, rel in base_rows} != expected_base:
        raise VerificationError("paired repair base manifest differs from the exact 180-file predecessor plus JSON owner")
    final_stage = repair_stage
    final_rows = _parse_sha_manifest(
        payload / final_stage["file"],
        "compile repair final",
        path_order=final_stage.get("path_order", "components"),
    )
    final_json_hash = dict((rel, digest) for digest, rel in final_rows).get(extra["path"])
    if {rel for _, rel in final_rows} != set(expected_base) or final_json_hash != extra["final_sha256"]:
        raise VerificationError("compile repair final manifest changed the paired base path set")

    cumulative_base = EXPECTED["cumulative_base"]
    if cumulative_base.get("path_order") != "lexical" or cumulative_base.get("paired_after") != "compile-repairs-v1":
        raise VerificationError("cumulative paired base must be lexical and follow compile-repairs-v1")
    if cumulative_base.get("count") != repair_stage["count"]:
        raise VerificationError("cumulative paired base count differs from compile-repairs-v1 final")
    cumulative_base_path = _path_without_symlinks(payload, cumulative_base["file"], "cumulative paired base")
    repair_final_path = _path_without_symlinks(payload, repair_stage["file"], "compile repair final")
    if cumulative_base_path.read_bytes() != repair_final_path.read_bytes():
        raise VerificationError("cumulative paired base manifest is not byte-identical to the compile repair final manifest")
    cumulative_base_rows = _parse_sha_manifest(cumulative_base_path, "cumulative paired base", path_order="lexical")
    if len(cumulative_base_rows) != cumulative_base["count"] or {rel: digest for digest, rel in cumulative_base_rows} != {rel: digest for digest, rel in final_rows}:
        raise VerificationError("cumulative paired base differs from the exact compile repair final source")
    cumulative_stage = stage_by_name.get("cumulative-hir-v1")
    if cumulative_stage is None or cumulative_stage.get("path_order") != "lexical":
        raise VerificationError("cumulative final manifest must use its pinned lexical path ordering")
    cumulative_final_path = _path_without_symlinks(payload, cumulative_stage["file"], "cumulative HIR final")
    cumulative_rows = _parse_sha_manifest(
        cumulative_final_path,
        "cumulative HIR final",
        path_order="lexical",
    )
    cumulative_json_hash = dict((rel, digest) for digest, rel in cumulative_rows).get(extra["path"])
    if {rel for _, rel in cumulative_rows} != {rel for _, rel in cumulative_base_rows} or cumulative_json_hash != extra["final_sha256"]:
        raise VerificationError("cumulative HIR final manifest changed the paired base path set or pinned JSON hash")

    next_base = EXPECTED["next_hir_base"]
    if next_base.get("path_order") != "lexical" or next_base.get("paired_after") != "cumulative-hir-v1":
        raise VerificationError("next HIR paired base must be lexical and immediately follow cumulative-hir-v1")
    next_index = EXPECTED["patch_order"].index("next-hir-v1")
    if next_index == 0 or EXPECTED["patch_order"][next_index - 1] != next_base["paired_after"]:
        raise VerificationError("next HIR paired base does not immediately precede its patch stage")
    next_stage = stage_by_name.get("next-hir-v1")
    if next_stage is None or next_stage.get("path_order") != "lexical" or next_stage.get("extra_paths") != [extra["path"]]:
        raise VerificationError("next HIR final manifest must preserve lexical ordering and the stage-scoped JSON owner")
    if next_base.get("count") != cumulative_stage["count"] or next_stage.get("count") != next_base["count"]:
        raise VerificationError("next HIR paired base and final counts must match the cumulative HIR stage")
    next_base_path = _path_without_symlinks(payload, next_base["file"], "next HIR paired base")
    if not next_base_path.is_file():
        raise VerificationError(f"next HIR paired base manifest missing: {next_base['file']}")
    if next_base_path.read_bytes() != cumulative_final_path.read_bytes():
        raise VerificationError("next HIR paired base manifest is not byte-identical to the cumulative HIR final manifest")
    next_base_rows = _parse_sha_manifest(next_base_path, "next HIR paired base", path_order="lexical")
    if len(next_base_rows) != next_base["count"] or {rel: digest for digest, rel in next_base_rows} != {rel: digest for digest, rel in cumulative_rows}:
        raise VerificationError("next HIR paired base differs from the exact cumulative HIR final source")
    next_base_map = {rel: digest for digest, rel in next_base_rows}
    next_final_path = _path_without_symlinks(payload, next_stage["file"], "next HIR final")
    next_final_rows = _parse_sha_manifest(next_final_path, "next HIR final", path_order="lexical")
    next_final_map = {rel: digest for digest, rel in next_final_rows}
    if len(next_final_rows) != next_stage["count"] or set(next_final_map) != set(next_base_map) or next_final_map.get(extra["path"]) != extra["final_sha256"]:
        raise VerificationError("next HIR final manifest changed the paired base path set or pinned JSON hash")

    wire_base = EXPECTED.get("forced_root_wire_base")
    wire_stage = stage_by_name.get("forced-root-wire-v1")
    if not isinstance(wire_base, dict) or wire_stage is None:
        raise VerificationError("forced-root wire paired base or final stage is missing")
    if wire_base.get("path_order") != "lexical" or wire_base.get("paired_after") != "next-hir-v1":
        raise VerificationError("forced-root wire paired base must be lexical and immediately follow next-hir-v1")
    wire_index = EXPECTED["patch_order"].index("forced-root-wire-v1")
    if wire_index == 0 or EXPECTED["patch_order"][wire_index - 1] != wire_base["paired_after"]:
        raise VerificationError("forced-root wire paired base does not immediately precede its patch stage")
    if wire_stage.get("path_order") != "lexical" or wire_stage.get("extra_paths") != FORCED_ROOT_STAGE_EXTRA_PATHS:
        raise VerificationError("forced-root wire final manifest must use lexical order and its exact stage-scoped paths")
    if wire_stage.get("count") != wire_base.get("count") or wire_base.get("count") != len(next_final_map) + len(FORCED_ROOT_PROVIDER_PATHS):
        raise VerificationError("forced-root wire paired base count must add exactly two QuickJS providers")
    if wire_base.get("file") != "forced-root-wire-v1-base.sha256" or wire_stage.get("file") != "forced-root-wire-v1.sha256":
        raise VerificationError("forced-root wire paired input filenames differ from the pinned contract")
    if wire_base.get("sha256") != EXPECTED["artifacts"].get(wire_base["file"]):
        raise VerificationError("forced-root wire paired-base descriptor is stale relative to its artifact pin")
    if wire_stage.get("sha256") != EXPECTED["artifacts"].get(wire_stage["file"]):
        raise VerificationError("forced-root wire final descriptor is stale relative to its artifact pin")
    wire_base_path = _path_without_symlinks(payload, wire_base["file"], "forced-root wire paired base")
    wire_stage_path = _path_without_symlinks(payload, wire_stage["file"], "forced-root wire final")
    if _sha(wire_base_path.read_bytes()) != wire_base["sha256"] or _sha(wire_stage_path.read_bytes()) != wire_stage["sha256"]:
        raise VerificationError("forced-root wire paired base or final manifest hash differs from its descriptor")
    wire_owners = wire_base.get("extra_owners")
    if (
        not isinstance(wire_owners, list)
        or len(wire_owners) != len(FORCED_ROOT_PROVIDER_PATHS)
        or any(not isinstance(owner, dict) for owner in wire_owners)
        or [owner.get("path") for owner in wire_owners] != FORCED_ROOT_PROVIDER_PATHS
    ):
        raise VerificationError("forced-root wire paired base must pin exactly the two QuickJS provider owners")
    wire_base_rows = _parse_sha_manifest(wire_base_path, "forced-root wire paired base", path_order="lexical")
    wire_final_rows = _parse_sha_manifest(wire_stage_path, "forced-root wire final", path_order="lexical")
    wire_base_map = {rel: digest for digest, rel in wire_base_rows}
    wire_final_map = {rel: digest for digest, rel in wire_final_rows}
    expected_wire_base = dict(next_final_map)
    expected_wire_final = dict(next_final_map)
    for owner in wire_owners:
        rel = owner["path"]
        if rel in expected_wire_base:
            raise VerificationError(f"forced-root wire provider already exists in the next HIR final: {rel}")
        base_path = _path_without_symlinks(baseline, rel, "forced-root wire provider")
        if not base_path.is_file() or base_path.is_symlink():
            raise VerificationError(f"forced-root wire provider is missing or not a regular baseline file: {rel}")
        base_bytes = base_path.read_bytes()
        if _sha(base_bytes) != owner["sha256"] or _git_blob_sha1(base_bytes) != owner["git_blob_sha1"]:
            raise VerificationError(f"forced-root wire provider SHA-256 or Git blob pin mismatch for {rel}")
        expected_wire_base[rel] = owner["sha256"]
        expected_wire_final[rel] = owner["final_sha256"]
        if net.get(rel) != "modified":
            raise VerificationError(f"forced-root wire provider must be a modified net owner: {rel}")
    if len(wire_base_rows) != wire_base["count"] or wire_base_map != expected_wire_base:
        raise VerificationError("forced-root wire paired base differs from the exact next HIR final plus two pinned baseline providers")
    if len(wire_final_rows) != wire_stage["count"] or wire_final_map != expected_wire_final:
        raise VerificationError("forced-root wire final changed prior-stage owners or differs from the two pinned provider finals")
    dependency_inputs = _verify_dependency_compile_inputs(payload, stage_by_name, net)
    std_compile_inputs = _verify_std_compile_inputs(payload, baseline, stage_by_name, net)
    quickjs_runtime_inputs = _verify_quickjs_runtime_inputs(payload, baseline, stage_by_name, net)
    quickjs_api_followup_inputs = _verify_quickjs_api_followup_inputs(payload, baseline, stage_by_name, net)
    return {
        "baseline": identity,
        "inputs": {**EXPECTED["payload_patches"], **EXPECTED["artifacts"]},
        "native_owner_count": len(owner_map),
        "net_owner_count": len(net),
        "dependency_compile": dependency_inputs,
        "std_compile": std_compile_inputs,
        "quickjs_runtime_compile": quickjs_runtime_inputs,
        "quickjs_api_followup": quickjs_api_followup_inputs,
        "patch_order": list(EXPECTED["patch_order"]),
    }


def _inventory(tree: Path) -> dict[str, tuple[str, int, str]]:
    """Hash full checkout content while excluding only Git metadata."""
    inventory: dict[str, tuple[str, int, str]] = {}
    root = tree.resolve()
    for current, dirs, files in os.walk(root, topdown=True, followlinks=False):
        current_path = Path(current)
        if current_path == root:
            dirs[:] = [name for name in dirs if name != ".git"]
        for name in list(dirs):
            path = current_path / name
            if path.is_symlink():
                rel = path.relative_to(root).as_posix()
                target = os.readlink(path).encode("utf-8", errors="surrogateescape")
                inventory[rel] = ("symlink", stat.S_IMODE(path.lstat().st_mode), _sha(target))
                dirs.remove(name)
        for name in files:
            path = current_path / name
            rel = path.relative_to(root).as_posix()
            mode = stat.S_IMODE(path.lstat().st_mode)
            if path.is_symlink():
                target = os.readlink(path).encode("utf-8", errors="surrogateescape")
                inventory[rel] = ("symlink", mode, _sha(target))
            elif path.is_file():
                inventory[rel] = ("file", mode, _sha(path.read_bytes()))
            else:
                raise VerificationError(f"checkout contains unsupported special file: {rel}")
    return inventory


def _verify_net_inventory(baseline: Path, candidate: Path, payload: Path) -> dict[str, Any]:
    before = _inventory(baseline)
    after = _inventory(candidate)
    roster = _read_net_roster(payload)
    actual: dict[str, str] = {}
    for rel in sorted(set(before) | set(after)):
        b = before.get(rel)
        c = after.get(rel)
        if b == c:
            continue
        if b is None:
            actual[rel] = "new"
        elif c is None:
            actual[rel] = "deleted"
        else:
            actual[rel] = "modified"
    if actual != roster:
        unexpected = sorted(set(actual) - set(roster))
        missing = sorted(set(roster) - set(actual))
        changed = sorted(path for path in set(actual) & set(roster) if actual[path] != roster[path])
        detail = []
        if unexpected:
            detail.append(f"unexpected owner {unexpected[0]} ({actual[unexpected[0]]})")
        if missing:
            detail.append(f"pinned owner was unchanged or absent {missing[0]} ({roster[missing[0]]})")
        if changed:
            detail.append(f"owner status differs for {changed[0]}: {actual[changed[0]]} vs {roster[changed[0]]}")
        raise VerificationError("final net-owner roster mismatch: " + "; ".join(detail))
    for rel, status in roster.items():
        b, c = before.get(rel), after.get(rel)
        if status == "modified":
            if b is None or c is None:
                raise VerificationError(f"modified owner missing on one side: {rel}")
            if b[0] != "file" or c[0] != "file" or b[1] != c[1]:
                raise VerificationError(f"modified owner changed file type or mode: {rel}")
        elif status == "new" and (b is not None or c is None or c[0] != "file"):
            raise VerificationError(f"new owner is not a fresh regular file: {rel}")
    for rel in EXPECTED["control_owners"]:
        entry = after.get(rel)
        if entry is None or entry[0] != "file" or entry[1] != 0o644:
            raise VerificationError(f"added control owner must be a regular 100644 file: {rel}")
    return {"changed_count": len(actual), "modified": sum(v == "modified" for v in actual.values()), "new": sum(v == "new" for v in actual.values())}


def check_required_controls(tree: Path, manifest_paths: set[str] | None = None) -> dict[str, Any]:
    """Require candidate control files and every pinned include target to exist."""
    final_manifest = EXPECTED["stage_manifests"][-1]["file"]
    if manifest_paths is None:
        final_stage = EXPECTED["stage_manifests"][-1]
        rows = _parse_sha_manifest(MODULE_DIR / final_manifest, "final control manifest", path_order=final_stage.get("path_order", "components"))
        manifest_paths = {rel for _, rel in rows}
    owners = EXPECTED["control_owners"]
    for rel in owners:
        path = _path_without_symlinks(tree, rel, "control owner")
        if rel not in manifest_paths or not path.is_file():
            raise VerificationError(f"required control owner missing from candidate: {rel}")
    for owner, includes in EXPECTED["required_includes"].items():
        path = _path_without_symlinks(tree, owner, "include owner")
        if owner not in manifest_paths or not path.is_file():
            raise VerificationError(f"required include owner missing from candidate: {owner}")
        text = path.read_text(encoding="utf-8")
        declared = set(re.findall(r'include!\s*\(\s*"([^"]+)"\s*\)', text))
        for include in includes:
            if include not in declared:
                raise VerificationError(f"required include missing from {owner}: {include}")
            include_rel = (PurePosixPath(owner).parent / include).as_posix()
            _safe_rel(include_rel, "include target")
            include_path = _path_without_symlinks(tree, include_rel, "include target")
            if include_rel not in manifest_paths or not include_path.is_file():
                raise VerificationError(f"missing include/control owner: {include_rel}")
    return {"control_owner_count": len(owners), "include_owner_count": len(EXPECTED["required_includes"])}


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    fd, temp_name = tempfile.mkstemp(prefix=path.name + ".", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_name, path)
    finally:
        try:
            os.unlink(temp_name)
        except FileNotFoundError:
            pass


def _evidence(evidence: Path, value: dict[str, Any]) -> None:
    try:
        _write_json(evidence / "reconstruction.json", value)
    except Exception as exc:
        raise EvidenceError(f"required reconstruction evidence write failed at {evidence}: {exc}") from exc


def _apply_patch(tree: Path, patch: Path) -> dict[str, Any]:
    for args in (("apply", "--check", str(patch)), ("apply", str(patch))):
        result = subprocess.run(
            ["git", "-C", str(tree), *args],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            raise VerificationError(
                f"git {' '.join(args[:2])} failed for {patch.name} ({result.returncode}): "
                f"{result.stderr.strip()}"
            )
    return {"patch": patch.name, "check": "passed", "apply": "passed"}


def _patch_path(baseline: Path, payload: Path, name: str) -> Path:
    if name == "original-native":
        rel = EXPECTED["source_files"]["original_patch"]["path"]
        return _path_without_symlinks(baseline, rel, "original patch")
    if name == "scope-v5":
        rel = EXPECTED["source_files"]["scope_patch"]["path"]
        return _path_without_symlinks(baseline, rel, "scope-v5 patch")
    payload_name = {
        "discard-v2": "discard-v2.patch",
        "return-v1": "return-v1.patch",
        "compile-repairs-v1": "compile-repairs-v1.patch",
        "cumulative-hir-v1": "cumulative-hir-v1.patch",
        "next-hir-v1": "next-hir-v1.patch",
        "forced-root-wire-v1": "forced-root-wire-v1.patch",
        "dependency-compile-v1": "dependency-compile-v1.patch",
        "std-compile-v1": "std-compile-v1.patch",
        "quickjs-runtime-compile-v1": "quickjs-runtime-compile-v1.patch",
        "quickjs-api-followup-v1": "quickjs-api-followup-v1.patch",
    }.get(name)
    if not payload_name:
        raise VerificationError(f"unknown patch stage: {name}")
    return _path_without_symlinks(payload, payload_name, f"{name} patch")


def verify_prepared(baseline: Path, candidate: Path, payload: Path) -> dict[str, Any]:
    """Revalidate identities for a prepared candidate without applying patches again."""
    _validate_independent_paths(baseline, candidate, payload)
    inputs = verify_inputs(baseline, payload)
    if not candidate.is_dir() or candidate.is_symlink():
        raise VerificationError(f"prepared candidate is missing or symlinked: {candidate}")
    final = EXPECTED["stage_manifests"][-1]
    stage = verify_stage(
        candidate,
        payload / final["file"],
        final["count"],
        final.get("extra_paths", []),
        final.get("path_order", "components"),
    )
    controls = check_required_controls(candidate, set(stage["files"]))
    net = _verify_net_inventory(baseline, candidate, payload)
    return {"inputs": inputs, "final_stage": stage, "controls": controls, "net": net}


def reconstruct(baseline: Path, candidate: Path, payload: Path, evidence: Path) -> dict[str, Any]:
    """Reconstruct the hash-pinned source stages on an untouched full checkout."""
    state: dict[str, Any] = {"status": "running", "baseline": str(baseline), "candidate": str(candidate), "stages": []}
    _validate_independent_paths(baseline, candidate, payload, evidence)
    try:
        input_result = verify_inputs(baseline, payload)
        state["input_validation"] = input_result
        if candidate.exists() or candidate.is_symlink():
            raise VerificationError(f"candidate destination must not already exist: {candidate}")
        baseline_abs = baseline.resolve()
        candidate_parent = candidate.parent.resolve()
        if not candidate_parent.is_dir():
            raise VerificationError(f"candidate parent directory does not exist: {candidate_parent}")
        def ignore_git_at_checkout_root(directory: str, names: list[str]) -> set[str]:
            return {".git"} if Path(directory).resolve() == baseline_abs and ".git" in names else set()

        shutil.copytree(baseline, candidate, symlinks=True, ignore=ignore_git_at_checkout_root)
        baseline_before = _inventory(baseline)
        if _inventory(candidate) != baseline_before:
            raise VerificationError("full candidate copy differs from the clean baseline before applying overlays")

        if [stage["name"] for stage in EXPECTED["stage_manifests"]] != EXPECTED["patch_order"]:
            raise VerificationError("patch order differs from immutable stage order")
        stage_by_name = {stage["name"]: stage for stage in EXPECTED["stage_manifests"]}
        for stage in EXPECTED["stage_manifests"]:
            patch = _patch_path(baseline, payload, stage["name"])
            if stage["name"] == "compile-repairs-v1":
                repair_base = EXPECTED["repair_base"]
                paired = verify_stage(
                    candidate,
                    payload / repair_base["file"],
                    repair_base["count"],
                    [repair_base["extra_owner"]["path"]],
                    repair_base.get("path_order", "components"),
                )
                state["paired_repair_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "extra_owner": repair_base["extra_owner"]["path"],
                }
            elif stage["name"] == "cumulative-hir-v1":
                cumulative_base = EXPECTED["cumulative_base"]
                repair_stage = next((item for item in EXPECTED["stage_manifests"] if item["name"] == cumulative_base["paired_after"]), None)
                if repair_stage is None or not state["stages"] or state["stages"][-1]["stage"] != repair_stage["name"]:
                    raise VerificationError("cumulative HIR paired base is not immediately after the named repair final")
                paired = verify_stage(
                    candidate,
                    payload / cumulative_base["file"],
                    cumulative_base["count"],
                    [EXPECTED["repair_base"]["extra_owner"]["path"]],
                    cumulative_base.get("path_order", "components"),
                )
                previous = state["stages"][-1]
                if paired["manifest_sha256"] != previous["manifest_sha256"] or paired["count"] != previous["count"]:
                    raise VerificationError("cumulative paired base does not equal the immediately preceding compile repair final")
                state["paired_cumulative_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": repair_stage["name"],
                    "extra_owner": EXPECTED["repair_base"]["extra_owner"]["path"],
                }
            elif stage["name"] == "next-hir-v1":
                next_base = EXPECTED["next_hir_base"]
                paired_stage_name = next_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("next HIR paired base is not immediately after the named cumulative HIR final")
                paired = verify_stage(
                    candidate,
                    payload / next_base["file"],
                    next_base["count"],
                    [EXPECTED["repair_base"]["extra_owner"]["path"]],
                    next_base.get("path_order", "components"),
                )
                previous = state["stages"][-1]
                if paired["manifest_sha256"] != previous["manifest_sha256"] or paired["count"] != previous["count"]:
                    raise VerificationError("next HIR paired base does not equal the immediately preceding cumulative HIR final")
                state["paired_next_hir_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_owner": EXPECTED["repair_base"]["extra_owner"]["path"],
                }
            elif stage["name"] == "forced-root-wire-v1":
                wire_base = EXPECTED["forced_root_wire_base"]
                paired_stage_name = wire_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("forced-root wire paired base is not immediately after the named next HIR final")
                paired = verify_stage(
                    candidate,
                    payload / wire_base["file"],
                    wire_base["count"],
                    FORCED_ROOT_STAGE_EXTRA_PATHS,
                    wire_base.get("path_order", "components"),
                )
                state["paired_forced_root_wire_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_owners": list(FORCED_ROOT_PROVIDER_PATHS),
                }
            elif stage["name"] == "dependency-compile-v1":
                dependency_base = EXPECTED["dependency_compile_base"]
                paired_stage_name = dependency_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("dependency compile paired base is not immediately after the named wire final")
                paired = verify_stage(
                    candidate,
                    payload / dependency_base["file"],
                    dependency_base["count"],
                    FORCED_ROOT_STAGE_EXTRA_PATHS,
                    dependency_base.get("path_order", "components"),
                )
                previous = state["stages"][-1]
                if paired["manifest_sha256"] != previous["manifest_sha256"] or paired["count"] != previous["count"]:
                    raise VerificationError("dependency compile paired base does not equal the immediately preceding wire final")
                state["paired_dependency_compile_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_paths": list(FORCED_ROOT_STAGE_EXTRA_PATHS),
                }
            elif stage["name"] == "std-compile-v1":
                std_base = EXPECTED["std_compile_base"]
                paired_stage_name = std_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("std compile paired base is not immediately after the dependency final")
                previous = stage_by_name[paired_stage_name]
                previous_result = verify_stage(
                    candidate,
                    payload / previous["file"],
                    previous["count"],
                    previous.get("extra_paths", []),
                    previous.get("path_order", "components"),
                )
                if previous_result["manifest_sha256"] != state["stages"][-1]["manifest_sha256"]:
                    raise VerificationError("std compile predecessor no longer matches the reconstructed dependency final")
                paired = verify_stage(
                    candidate,
                    payload / std_base["file"],
                    std_base["count"],
                    STD_COMPILE_EXTRA_PATHS,
                    std_base.get("path_order", "components"),
                )
                state["paired_std_compile_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_paths": list(STD_COMPILE_EXTRA_PATHS),
                    "http_baseline_sha256": STD_HTTP_BASE_SHA256,
                    "http_baseline_git_blob_sha1": STD_HTTP_BASE_GIT_BLOB_SHA1,
                }
            elif stage["name"] == "quickjs-runtime-compile-v1":
                quickjs_base = EXPECTED["quickjs_runtime_compile_base"]
                paired_stage_name = quickjs_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("QuickJS/runtime paired base is not immediately after the std final")
                previous = stage_by_name[paired_stage_name]
                previous_result = verify_stage(
                    candidate,
                    payload / previous["file"],
                    previous["count"],
                    previous.get("extra_paths", []),
                    previous.get("path_order", "components"),
                )
                if previous_result["manifest_sha256"] != state["stages"][-1]["manifest_sha256"]:
                    raise VerificationError("QuickJS/runtime predecessor no longer matches the reconstructed std final")
                paired = verify_stage(
                    candidate,
                    payload / quickjs_base["file"],
                    quickjs_base["count"],
                    QUICKJS_RUNTIME_EXTRA_PATHS,
                    quickjs_base["path_order"],
                )
                state["paired_quickjs_runtime_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_owner": QUICKJS_WASM_PATH,
                    "wasm_baseline_sha256": QUICKJS_WASM_BASE_SHA256,
                    "wasm_baseline_git_blob_sha1": QUICKJS_WASM_BASE_GIT_BLOB_SHA1,
                }
            elif stage["name"] == "quickjs-api-followup-v1":
                api_base = EXPECTED["quickjs_api_followup_base"]
                paired_stage_name = api_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("QuickJS API follow-up paired base is not immediately after the QuickJS/runtime final")
                previous = stage_by_name[paired_stage_name]
                previous_result = verify_stage(
                    candidate,
                    payload / previous["file"],
                    previous["count"],
                    previous.get("extra_paths", []),
                    previous.get("path_order", "components"),
                )
                if previous_result["manifest_sha256"] != state["stages"][-1]["manifest_sha256"]:
                    raise VerificationError("QuickJS API follow-up predecessor no longer matches the reconstructed QuickJS/runtime final")
                paired = verify_stage(
                    candidate,
                    payload / api_base["file"],
                    api_base["count"],
                    QUICKJS_API_FOLLOWUP_EXTRA_PATHS,
                    api_base["path_order"],
                )
                state["paired_quickjs_api_followup_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_owner": QUICKJS_TESTS_PATH,
                    "tests_baseline_sha256": QUICKJS_TESTS_BASE_SHA256,
                    "tests_baseline_git_blob_sha1": QUICKJS_TESTS_BASE_GIT_BLOB_SHA1,
                }
            applied = _apply_patch(candidate, patch)
            manifest = payload / stage["file"]
            result = verify_stage(
                candidate,
                manifest,
                stage["count"],
                stage.get("extra_paths", []),
                stage.get("path_order", "components"),
            )
            row = {**applied, "stage": stage["name"], "count": result["count"], "manifest_sha256": result["manifest_sha256"]}
            state["stages"].append(row)
            state["last_good_stage"] = stage["name"]

        final_stage = EXPECTED["stage_manifests"][-1]
        final_files = set(
            row[1]
            for row in _parse_sha_manifest(
                payload / final_stage["file"],
                "final manifest",
                path_order=final_stage.get("path_order", "components"),
            )
        )
        controls = check_required_controls(candidate, final_files)
        net = _verify_net_inventory(baseline, candidate, payload)
        state.update({"status": "passed", "controls": controls, "net": net, "stage_counts": [x["count"] for x in state["stages"]]})
        _evidence(evidence, state)
        return state
    except EvidenceError:
        raise
    except Exception as exc:
        state["status"] = "failed"
        state["error"] = f"{type(exc).__name__}: {exc}"
        try:
            _evidence(evidence, state)
        except EvidenceError as write_exc:
            raise VerificationError(f"{state['error']}; {write_exc}") from exc
        if isinstance(exc, VerificationError):
            raise
        raise VerificationError(f"reconstruction failed: {exc}") from exc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--payload", type=Path, default=MODULE_DIR)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = reconstruct(args.baseline, args.candidate, args.payload, args.evidence)
    except VerificationError as exc:
        print(f"native candidate identity verification failed: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
