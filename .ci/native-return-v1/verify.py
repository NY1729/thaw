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
INTEGRATED_REPAIR_EXTRA_OWNERS = [
    {"path": "crates/thaw-bridge/src/bridge/classification.rs", "sha256": "02eb8290ce3ea489e663e046edf5691ce3b5bd73917e2734b24c66fe062cbc93", "git_blob_sha1": "3decfaf234527b9e2e0ed403893edcdd1e98e485", "final_sha256": "70ffb7d273d206b1b30bdaefb7a0658e76a144f4b747c9a6ab0fae0f02103c52"},
    {"path": "crates/thaw-bridge/src/bridge/dts/exports.rs", "sha256": "0344c3c12445bd738728f6a23e0080a2aa89c26d0344069bc8323f47a541ab4c", "git_blob_sha1": "5dabf58ceb0069f81fefec38d09f3a33d492daf8", "final_sha256": "12a34f556f429b7b59f886c8d390f82739905a52a0f151fcdfe687cee356d4d3"},
    {"path": "crates/thaw-bridge/src/bridge/dts/interfaces.rs", "sha256": "c670f2ee5c51026a0c3005d9c80ef55ae2f1be1393eb9f975599ea108abefb43", "git_blob_sha1": "d3f459e5a87e96893f95fd75c0d0ce64b2732699", "final_sha256": "59945dbe23067d664c7c6c12023c97498cc980a64abe1b102ef7c0d2eb16918d"},
    {"path": "crates/thaw-bridge/src/bridge/generation.rs", "sha256": "62b85553eb66b68c05cdcf0ddfe862605cb0db69ca379ab5314c571058bbe915", "git_blob_sha1": "3d53e52ae5312ccd1b7ae7b0a327b7f9d291552d", "final_sha256": "c72000d3c33b4b31ad34083f94f1d2db63f2b9a4ac67b5d4ba005405d0019dcf"},
    {"path": "crates/thaw-bridge/src/tests/diagnostics_and_classes.rs", "sha256": "ac2967c7fd3f9bcc5e1169f25d54a7e2bf34e7c52cb4449198b45d135120f3be", "git_blob_sha1": "c89f4fa72fdc1ad35b235bbb44005cfc85751ab1", "final_sha256": "39e3169af7ded62fab86ab9ec769f33a5fd94b0139559d7a70f8b9bbca6d301c"},
    {"path": "crates/thaw-bridge/src/tests/generic_types.rs", "sha256": "8f45f4f6b1f2903381da46c29ff94356b67632e1229e82e39077bde218c163fa", "git_blob_sha1": "e7cc60bdc19a07a679a18dbd2e080922aef69c30", "final_sha256": "e9ac22a235b5aca84828bf789486a1c62e42dca4f6aacc7b93c192e877b81db4"},
    {"path": "crates/thaw-bridge/src/tests/shims_and_bundles.rs", "sha256": "ec1b0d835d7ae3a65d43a6bb69ed6804aef369238ac3d216b2d287cdd92e1288", "git_blob_sha1": "f1935f9e94a17c040c5576e82c08a31327830af0", "final_sha256": "26b1b7187dc91333c1fa9ccfaf8084e6f1cb4b53e725b7f00c224862f0055041"},
    {"path": "crates/thaw-cli/src/module_graph.rs", "sha256": "eb355695ba186122d677854bbb456bc3e56bce6776613387ea44ccb55ba1a84f", "git_blob_sha1": "4341f92cecd1f067a505fe90a8ae0e930526e1aa", "final_sha256": "62f9d9f795bc74461993165047abe10cbfdbeb18c779a25835f4f2a3c41ab201"},
    {"path": "crates/thaw-cli/src/registry_integration/jit/control_flow.rs", "sha256": "6bd4eeffc54e1db641676fc7a3e4604d95cb3e62ea6606c27640d2c82229641b", "git_blob_sha1": "e98b7f20079f98cd914863bd83637cce2a107109", "final_sha256": "78d6acb00628190b388af2105fe51f02c074660f6be11b8f14407fc53edf4a3c"},
    {"path": "crates/thaw-cli/src/registry_integration/shim_support.rs", "sha256": "0493fcc27157fc342a39fb131e41f0945705c2c6e9f629ac4d5548108700e696", "git_blob_sha1": "5280eb9e2556e811dd57828b4d378d6b0fe2b01c", "final_sha256": "c075d54dccefd2142eb476cc8076d5be8694e7119ded08c6d4e827b18efcc407"},
    {"path": "crates/thaw-cli/src/tests/external_classes.rs", "sha256": "d02fdc4c1dce42d56d7fb15f0ab4a1ce9aefbe82cc660bc41a056b8b10cf3b0f", "git_blob_sha1": "6c56f380e6eea040c947c213719a670cf9104766", "final_sha256": "cbab0e91e9d9f7783767524f446ef94dc35356c413b740960da7cce3b42ed180"},
    {"path": "crates/thaw-cli/src/tests.rs", "sha256": "36f71b073d3fbf61a36872a66088cb80a2418765edf2f3bad1452d9cc6b5106f", "git_blob_sha1": "aab3b8a26d831b35f87c2ade0f20e0c436f8d2a3", "final_sha256": "03a2847952bf36cc861d6699516b8cc2d56d9658dad719fc919259ac01dffc1a"},
    {"path": "crates/thaw-napi/src/napi/classes.rs", "sha256": "7458044cd347d7834477ee0b90a5a688260e02f9a721b7bf5ca2740bfb31588c", "git_blob_sha1": "6cc69d47c43e615e2203a54aa2b29b8b11ee5202", "final_sha256": "4f5de66451dc8c16fdcf9e27543d577e1af3414fcb483a1774d994e0ad9b0bd8"},
    {"path": "crates/thaw-napi/src/napi/module_host.rs", "sha256": "e017ea5491d5bbb7587f6bbe45b2fbd6ab4c597d32e67f1901ee76749864f33c", "git_blob_sha1": "63d7f5e635ba1fde0c2ee4e26375e8fe6ef6d061", "final_sha256": "791ae0601e9f11118b0a2bdd21463cfb83f8f347cc348017f79a33a22936adf0"},
    {"path": "crates/thaw-napi/src/tests/async_runtime.rs", "sha256": "a4a975c0f3899ee3a11ef61cf7202dc1fb9f505c3cd8e09ca1239b77c3b8290b", "git_blob_sha1": "66a1974d8d8385fb10877a8d494f13f47dcf1bf5", "final_sha256": "070a5a87b8664c84b1193bf15cf12496b792dfea13ea92fdb88c1ae4b8946cb3"},
    {"path": "crates/thaw-napi/src/tests/handles.rs", "sha256": "b1280061c9204ce8cab1d2c1483b3d25de591c44b45831680ad4a2f57aac9250", "git_blob_sha1": "e1b32d224a47587feb6dfc76a8988ba816230ede", "final_sha256": "7e16cc1ee233828c67e4532ac5be3802353987a4022dda5eaa0fd39a3117b330"},
    {"path": "crates/thaw-napi/src/tests/objects_classes.rs", "sha256": "226fdb71d173e689e0f3140b1eedf6a30e02c3cc37da31cb540ae57f43d61304", "git_blob_sha1": "2136ae2bcda3b9f3c6431b88ee774f2c1f4e29f4", "final_sha256": "e1cbe2aad9c9b649698fee9ff3325df8f1ee68606c6faa8f7bb6e31dcc5f1f67"},
    {"path": "crates/thaw-napi/src/tests.rs", "sha256": "5551950bc17cc3b228016af3be8fe0596e5dd5d38eb811bc98f32573112c7bdb", "git_blob_sha1": "0a0981cda62adcc429d64c7f320a9500031bcfae", "final_sha256": "0efce312cad74e4923349f903de4552a382bdc95d10fb6055facd351885ba500"},
    {"path": "crates/thaw-quickjs/src/lib.rs", "sha256": "65848d5d51716015b3285c534cfc390a517676e74843f7127806188d0ab2556a", "git_blob_sha1": "c29813cae26b3eafe7eb92e8b3f2b67e60e0c3ff", "final_sha256": "b06e314ac6fca3cd81ca1100d5b6f51dec05d56bf873305f492523463ba7e85a"},
    {"path": "crates/thaw-quickjs/src/quickjs/filesystem.rs", "sha256": "877d2d08445bc94e6b2d1095cf8b40943e3a8d79840535961b3f951d65b944ed", "git_blob_sha1": "d8d660f06c282650e3efe392219e67509765f185", "final_sha256": "4677a999ad895ee2d190bd72b4e4374248136b766e8c643c7b4a58606a9e8463"},
    {"path": "crates/thaw-quickjs/src/quickjs/platform_globals/webassembly.js", "sha256": "e81b910bbec01ac145abc06521b664997310c3d63f4b27036297b5ebe29327f1", "git_blob_sha1": "4a30a650e48be04af5f190541d644d2793d515ce", "final_sha256": "0c4480cfe460d6747acffbecfb16c849ef3b5c12b878920b9e94b3ce1f8953da"},
    {"path": "crates/thaw-registry/src/registry/builtins/filesystem.rs", "sha256": "24c171163d7737d22ff46c59b7235ded6a3cd24bd8ffd704d4a96a62ddfa7f3b", "git_blob_sha1": "2bdaec73aaee7ae7b662ae36836e530fd9fffecb", "final_sha256": "c29ee7ecaf5587503b49dfd893de78f9f8f8ad47fa321a88aee60b73e603aa8b"},
    {"path": "crates/thaw-registry/src/registry/builtins/http/http1.rs", "sha256": "b2e1e01f2accf6b6d85170601c6abecd3ece8c0ee6ae5243476a8f741c68e125", "git_blob_sha1": "044885b373d0c6c067757d3bfb5a9cf8b06d5a39", "final_sha256": "a5a8f3fb2c05c27d8f95496e6dcb33a7c559096667302844d94f0154a6e002ab"},
    {"path": "crates/thaw-registry/src/registry/bundle/render.rs", "sha256": "73df0361495332c75d3b0f479c7114eeee1ecdd7b926dc6c4ea8370ff81747c8", "git_blob_sha1": "cab54090977a051a912987339835f6b4074bb6ed", "final_sha256": "73a4bc528fec004e410fff3804dae1ed4c95707c2a391e8404ea41db0665ca75"},
    {"path": "crates/thaw-registry/src/registry/bundle.rs", "sha256": "5aa0eae2f039291cb3ba708a1e5404434b34913935e4ebf92bdc81183f592ba9", "git_blob_sha1": "f14f23c11818b224335f22d8b3bb4460936ca7aa", "final_sha256": "f6854f6d93b3b13d7266e60ddcbdec3d1e4eedc29c67f64c54022ca63bca616d"},
    {"path": "crates/thaw-registry/src/registry/install/declarations.rs", "sha256": "433bd953f14da64721d8a12114c018645caec38442a2c7a5ac7c3c51342af2aa", "git_blob_sha1": "56a12dbf5a70b0b73df7ffe3ab936783d0c52b72", "final_sha256": "4e776b2dbbf15296a011dc77571bc874b8f59077b4e83287dd8d3fc300d9c9ef"},
    {"path": "crates/thaw-registry/src/tests/filesystem.rs", "sha256": "a074ce70485cfc71be385cadbfe608a4ae87714cefffc5a791fe2b2f5802fedc", "git_blob_sha1": "4e897fad33e26b3f19389d6194ef8cbf324a952a", "final_sha256": "5bf34341b10994b6b097d178692ff5f85fa3a7c421f5898998d9728838cdeb7c"},
    {"path": "crates/thaw-registry/src/tests/network.rs", "sha256": "4df1db3e772acf512bd322430c7bc26ee5a06371d3bd9b134a3b5f9d93d05af0", "git_blob_sha1": "c73acab6464646aa39939c247f6f9a56004b5e8e", "final_sha256": "0bf1f77533ce240a6ea2299135f4b5fae9740b43697454a20e7b276da8f3459e"},
    {"path": "crates/thaw-registry/src/tests/node_core.rs", "sha256": "83230904efea96fcae2dc2044ba5fc07ec13b631d0e50e1b573038bc00f3657c", "git_blob_sha1": "323c31a2d858adc0d6f670eb62a585a573384012", "final_sha256": "651372f55c8c0c247f2917711759260bf353a573cc680125ac14d84caf4978b6"},
    {"path": "crates/thaw-registry/src/tests/platform.rs", "sha256": "2194858509fc0d0acb4840b843dff68bb6c5c59e3fa62771bc3eb366e6041de3", "git_blob_sha1": "d12a88aca1041ade973d52d8ef48fba8d9429be1", "final_sha256": "2a55863725c929ac727942d61118fd360d9010881260bf2f9e2060f3f488bd4c"},
    {"path": "crates/thaw-registry/src/tests/stream_web.rs", "sha256": "a291f5eaff89a0d3a1c7431cb21c4f4aafcddd3479a44332ce6a93e2b0784796", "git_blob_sha1": "9d404bf2b3208d2543d85a06885ff5fd1a8acb22", "final_sha256": "9adce8bb9d8db0f1ab737190f467a9b20b753d7f7ec86f359fe7304e910c794b"},
    {"path": "crates/thaw-registry/src/tests.rs", "sha256": "982ac4ca35435172751d58aa6c5e666dc862d17707854c0757783bd222f71187", "git_blob_sha1": "c45a631d8b8f3f7e5e7d100ba3ca62f11b005022", "final_sha256": "4377fd3130884fdc160762d6c2490543a2f3ae561b9fc5913bb715c3f26e9779"},
]
INTEGRATED_REPAIR_EXTRA_PATHS = [*QUICKJS_API_FOLLOWUP_EXTRA_PATHS, *[owner["path"] for owner in INTEGRATED_REPAIR_EXTRA_OWNERS]]
INTEGRATED_REPAIR_SOURCE_PATHS = [
    "crates/thaw-arena/src/strings.rs",
    "crates/thaw-bridge/src/bridge/classification.rs",
    "crates/thaw-bridge/src/bridge/dts/exports.rs",
    "crates/thaw-bridge/src/bridge/dts/interfaces.rs",
    "crates/thaw-bridge/src/bridge/generation.rs",
    "crates/thaw-bridge/src/tests/diagnostics_and_classes.rs",
    "crates/thaw-bridge/src/tests/generic_types.rs",
    "crates/thaw-bridge/src/tests/shims_and_bundles.rs",
    "crates/thaw-cli/src/module_graph.rs",
    "crates/thaw-cli/src/registry_integration/jit/control_flow.rs",
    "crates/thaw-cli/src/registry_integration/shim_support.rs",
    "crates/thaw-cli/src/tests.rs",
    "crates/thaw-cli/src/tests/external_classes.rs",
    "crates/thaw-hir/src/lower/assignments/updates.rs",
    "crates/thaw-hir/src/lower/expressions/lowering.rs",
    "crates/thaw-hir/src/lower/generic_calls.rs",
    "crates/thaw-hir/src/lower/inference/types.rs",
    "crates/thaw-hir/src/lower/invocations/calls.rs",
    "crates/thaw-hir/src/lower/invocations/static_builtins.rs",
    "crates/thaw-hir/src/lower/module/helpers.rs",
    "crates/thaw-hir/src/lower/statements/lowering.rs",
    "crates/thaw-hir/src/lower/tests.rs",
    "crates/thaw-hir/src/lower/tests/classes.rs",
    "crates/thaw-hir/src/lower/tests/control_flow.rs",
    "crates/thaw-hir/src/lower/tests/types.rs",
    "crates/thaw-hir/src/lower/tests/values.rs",
    "crates/thaw-llvm/src/hir_codegen.rs",
    "crates/thaw-llvm/src/hir_codegen/async_frames/codegen.rs",
    "crates/thaw-llvm/src/hir_codegen/async_frames/planning/loops.rs",
    "crates/thaw-llvm/src/hir_codegen/collections.rs",
    "crates/thaw-llvm/src/hir_codegen/dynamic_host/callbacks.rs",
    "crates/thaw-llvm/src/hir_codegen/dynamic_host/napi.rs",
    "crates/thaw-llvm/src/hir_codegen/dynamic_host/quickjs.rs",
    "crates/thaw-llvm/src/hir_codegen/invocations/dynamic_calls.rs",
    "crates/thaw-llvm/src/hir_codegen/invocations/json_calls.rs",
    "crates/thaw-llvm/src/hir_codegen/json_bridge/decoding.rs",
    "crates/thaw-llvm/src/hir_codegen/json_values.rs",
    "crates/thaw-llvm/src/hir_codegen/runtime_declarations.rs",
    "crates/thaw-llvm/src/hir_codegen/statements.rs",
    "crates/thaw-llvm/src/hir_codegen/values/closures.rs",
    "crates/thaw-llvm/src/hir_codegen/values/expressions.rs",
    "crates/thaw-napi/src/napi/classes.rs",
    "crates/thaw-napi/src/napi/module_host.rs",
    "crates/thaw-napi/src/tests.rs",
    "crates/thaw-napi/src/tests/async_runtime.rs",
    "crates/thaw-napi/src/tests/handles.rs",
    "crates/thaw-napi/src/tests/objects_classes.rs",
    "crates/thaw-quickjs/src/lib.rs",
    "crates/thaw-quickjs/src/quickjs/api.rs",
    "crates/thaw-quickjs/src/quickjs/filesystem.rs",
    "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js",
    "crates/thaw-quickjs/src/quickjs/platform_globals/webassembly.js",
    "crates/thaw-quickjs/src/tests.rs",
    "crates/thaw-registry/src/registry/builtins/filesystem.rs",
    "crates/thaw-registry/src/registry/builtins/http/http1.rs",
    "crates/thaw-registry/src/registry/bundle.rs",
    "crates/thaw-registry/src/registry/bundle/render.rs",
    "crates/thaw-registry/src/registry/install/declarations.rs",
    "crates/thaw-registry/src/tests.rs",
    "crates/thaw-registry/src/tests/filesystem.rs",
    "crates/thaw-registry/src/tests/network.rs",
    "crates/thaw-registry/src/tests/node_core.rs",
    "crates/thaw-registry/src/tests/platform.rs",
    "crates/thaw-registry/src/tests/stream_web.rs",
    "crates/thaw-std/src/json.rs",
]
INTEGRATED_REPAIR_V2_EXTRA_OWNERS = [
    {"path": "crates/thaw-cli/src/tests/registry_jit_primitives.rs", "sha256": "35aa62e61b5a708859438b43579355a7cc00922c9be4fc511d981fa6bedcc8a1", "git_blob_sha1": "064825f07bada1377e742e37a41c2d0bca109ee0", "final_sha256": "db3c62537e3460b46963512217ac9c2aeb86d5600bbdcabced4d21966da57941"},
    {"path": "crates/thaw-std/src/lib.rs", "sha256": "0b76a2239a2013d9f80820826df3054036f410caa4a7073109244fceaef0e040", "git_blob_sha1": "9bbdb2f0706c5ea564e96c21a7c73be7859ac33c", "final_sha256": "39914a8fc68de17afa706dcb47ae8dfac92e354fddcaf76e3d199d16e6abfa22"},
]
INTEGRATED_REPAIR_V2_ADDED_PATHS = [
    "crates/thaw-std/src/inspect.rs",
    "crates/thaw-std/src/inspect_corpus.json",
    "crates/thaw-std/src/inspect_width.rs",
]
INTEGRATED_REPAIR_V2_BASE_EXTRA_PATHS = [*INTEGRATED_REPAIR_EXTRA_PATHS, *[owner["path"] for owner in INTEGRATED_REPAIR_V2_EXTRA_OWNERS]]
INTEGRATED_REPAIR_V2_FINAL_EXTRA_PATHS = [*INTEGRATED_REPAIR_V2_BASE_EXTRA_PATHS, *INTEGRATED_REPAIR_V2_ADDED_PATHS]
INTEGRATED_REPAIR_V2_SOURCE_PATHS = [
    "crates/thaw-cli/src/tests/registry_jit_primitives.rs",
    "crates/thaw-hir/src/hir/ir.rs",
    "crates/thaw-hir/src/lower/expressions/coercions.rs",
    "crates/thaw-hir/src/lower/objects.rs",
    "crates/thaw-hir/src/lower/tests/control_flow.rs",
    "crates/thaw-hir/src/lower/tests/values.rs",
    "crates/thaw-llvm/src/hir_codegen.rs",
    "crates/thaw-llvm/src/hir_codegen/async_frames/codegen.rs",
    "crates/thaw-llvm/src/hir_codegen/async_frames/planning/analysis.rs",
    "crates/thaw-llvm/src/hir_codegen/async_frames/planning/plan.rs",
    "crates/thaw-llvm/src/hir_codegen/async_frames/planning/try.rs",
    "crates/thaw-llvm/src/hir_codegen/console.rs",
    "crates/thaw-llvm/src/hir_codegen/invocations/calls.rs",
    "crates/thaw-llvm/src/hir_codegen/invocations/dynamic_calls.rs",
    "crates/thaw-llvm/src/hir_codegen/json_bridge/encoding.rs",
    "crates/thaw-llvm/src/hir_codegen/runtime_declarations.rs",
    "crates/thaw-llvm/src/hir_codegen/statements.rs",
    "crates/thaw-llvm/src/hir_codegen/tests/async/eval_then_returns.rs",
    "crates/thaw-llvm/src/hir_codegen/tests/async/union_discard_eval_then.rs",
    "crates/thaw-llvm/src/hir_codegen/tests/collections.rs",
    "crates/thaw-llvm/src/hir_codegen/tests/core.rs",
    "crates/thaw-llvm/src/hir_codegen/tests/native_builtins.rs",
    "crates/thaw-llvm/src/hir_codegen/tests/types.rs",
    "crates/thaw-llvm/src/hir_codegen/values/expressions.rs",
    "crates/thaw-std/src/json.rs",
    "crates/thaw-std/src/lib.rs",
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
    "net-owners.txt": "8b4dbac6c20fa735d491b128510ec2cc9d7ee70578746555550a54a70809afad",
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
    "sha256": "8b4dbac6c20fa735d491b128510ec2cc9d7ee70578746555550a54a70809afad",
    "count": 117,
    "modified": 109,
    "new": 8,
}
EXPECTED["patch_order"].append("quickjs-api-followup-v1")
EXPECTED["payload_patches"]["integrated-repair-v1.patch"] = "81cafd231902115b5c690717df23e0e8f62e04a3b2ca5558827223fe9b566bb8"
EXPECTED["artifacts"].update({
    "integrated-repair-v1-base.sha256": "eb5bce4fc8202ef190456bd3336ce3f18f832a48024322ebcf80be6d82c39edb",
    "integrated-repair-v1.sha256": "9ea9050c4d053a4c620ae259025404042f9c30e0c63127fd87d892bd15239902",
})
EXPECTED["stage_manifests"].append({
    "name": "integrated-repair-v1",
    "file": "integrated-repair-v1.sha256",
    "sha256": "9ea9050c4d053a4c620ae259025404042f9c30e0c63127fd87d892bd15239902",
    "count": 218,
    "extra_paths": INTEGRATED_REPAIR_EXTRA_PATHS,
    "path_order": "lexical",
})
EXPECTED["integrated_repair_base"] = {
    "file": "integrated-repair-v1-base.sha256",
    "sha256": "eb5bce4fc8202ef190456bd3336ce3f18f832a48024322ebcf80be6d82c39edb",
    "count": 218,
    "path_order": "lexical",
    "paired_after": "quickjs-api-followup-v1",
    "extra_owners": INTEGRATED_REPAIR_EXTRA_OWNERS,
}
EXPECTED["integrated_repair_source_paths"] = INTEGRATED_REPAIR_SOURCE_PATHS
EXPECTED["patch_order"].append("integrated-repair-v1")
EXPECTED["payload_patches"]["integrated-repair-v2.patch"] = "9291b525f2443fdbedbac6a0d8b44147672e623315f63ef6c7e3ba5d577c72f4"
EXPECTED["artifacts"].update({
    "integrated-repair-v2-base.sha256": "6130408ab8e64219ec095bdeb9a4cf0df3cdd2cc5e3916057af2ec94bee2f2e3",
    "integrated-repair-v2.sha256": "7ef304f8d7469d52d17080e7e380898cb79f5f8be349e1a96b4f4eea5dcfb78f",
})
EXPECTED["stage_manifests"].append({
    "name": "integrated-repair-v2",
    "file": "integrated-repair-v2.sha256",
    "sha256": "7ef304f8d7469d52d17080e7e380898cb79f5f8be349e1a96b4f4eea5dcfb78f",
    "count": 223,
    "extra_paths": INTEGRATED_REPAIR_V2_FINAL_EXTRA_PATHS,
    "path_order": "lexical",
})
EXPECTED["integrated_repair_v2_base"] = {
    "file": "integrated-repair-v2-base.sha256",
    "sha256": "6130408ab8e64219ec095bdeb9a4cf0df3cdd2cc5e3916057af2ec94bee2f2e3",
    "count": 220,
    "path_order": "lexical",
    "paired_after": "integrated-repair-v1",
    "extra_owners": INTEGRATED_REPAIR_V2_EXTRA_OWNERS,
    "added_paths": INTEGRATED_REPAIR_V2_ADDED_PATHS,
}
EXPECTED["integrated_repair_v2_source_paths"] = INTEGRATED_REPAIR_V2_SOURCE_PATHS
EXPECTED["patch_order"].append("integrated-repair-v2")
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
        INTEGRATED_REPAIR_EXTRA_PATHS,
        INTEGRATED_REPAIR_V2_BASE_EXTRA_PATHS,
        INTEGRATED_REPAIR_V2_FINAL_EXTRA_PATHS,
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
    quickjs_api_index = EXPECTED["patch_order"].index("quickjs-api-followup-v1")
    if (
        base.get("paired_after") != previous["name"]
        or quickjs_api_index == 0
        or EXPECTED["patch_order"][quickjs_api_index - 1] != "quickjs-runtime-compile-v1"
    ):
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


def _verify_integrated_repair_inputs(
    payload: Path, baseline: Path, stage_by_name: dict[str, dict[str, Any]], net: dict[str, str]
) -> dict[str, Any]:
    """Pin stage thirteen to stage twelve plus the baseline copies of its extra owners."""
    base = EXPECTED.get("integrated_repair_base")
    previous = stage_by_name.get("quickjs-api-followup-v1")
    stage = stage_by_name.get("integrated-repair-v1")
    if not isinstance(base, dict) or previous is None or stage is None:
        raise VerificationError("integrated repair paired base or stage is missing")
    order = EXPECTED["patch_order"]
    index = order.index("integrated-repair-v1")
    if base.get("paired_after") != previous["name"] or index == 0 or order[index - 1] != "quickjs-api-followup-v1":
        raise VerificationError("integrated repair paired base must immediately follow quickjs-api-followup-v1")
    if base.get("path_order") != "lexical" or previous.get("path_order") != "lexical":
        raise VerificationError("integrated repair base and predecessor must retain lexical path ordering")
    if previous.get("extra_paths") != QUICKJS_API_FOLLOWUP_EXTRA_PATHS or stage.get("extra_paths") != INTEGRATED_REPAIR_EXTRA_PATHS:
        raise VerificationError("integrated repair stage has an unexpected stage-scoped path set")
    owners = base.get("extra_owners")
    if not isinstance(owners, list) or [owner.get("path") for owner in owners] != INTEGRATED_REPAIR_EXTRA_PATHS[len(QUICKJS_API_FOLLOWUP_EXTRA_PATHS):]:
        raise VerificationError("integrated repair paired base extra owners differ from the pinned stage-scoped path set")
    source_paths = EXPECTED.get("integrated_repair_source_paths")
    if not isinstance(source_paths, list) or sorted(set(source_paths)) != sorted(source_paths):
        raise VerificationError("integrated repair source path pin is missing or not unique")
    extra_count = len(owners)
    if stage.get("path_order") != "lexical" or base.get("count") != previous["count"] + extra_count or stage.get("count") != base["count"]:
        raise VerificationError("integrated repair paired base and final must contain exactly the predecessor plus the baseline extra owners")
    if base.get("file") != "integrated-repair-v1-base.sha256" or stage.get("file") != "integrated-repair-v1.sha256":
        raise VerificationError("integrated repair paired input filenames differ from the pinned contract")
    for descriptor in (base, stage):
        if descriptor.get("sha256") != EXPECTED["artifacts"].get(descriptor["file"]):
            raise VerificationError(f"integrated repair manifest pin is stale for {descriptor['file']}")
    for owner in owners:
        owner_path = _path_without_symlinks(baseline, owner["path"], "integrated repair baseline extra owner")
        if not owner_path.is_file() or owner_path.is_symlink():
            raise VerificationError(f"integrated repair baseline extra owner is missing or not a regular file: {owner['path']}")
        owner_bytes = owner_path.read_bytes()
        if _sha(owner_bytes) != owner["sha256"] or _git_blob_sha1(owner_bytes) != owner["git_blob_sha1"]:
            raise VerificationError(f"baseline extra owner SHA-256 or Git blob identity mismatch: {owner['path']}")
    base_path = _path_without_symlinks(payload, base["file"], "integrated repair paired base")
    previous_path = _path_without_symlinks(payload, previous["file"], "integrated repair predecessor final")
    final_path = _path_without_symlinks(payload, stage["file"], "integrated repair final")
    previous_rows = _parse_sha_manifest(previous_path, "integrated repair predecessor final", path_order="lexical")
    base_rows = _parse_sha_manifest(base_path, "integrated repair paired base", path_order="lexical")
    final_rows = _parse_sha_manifest(final_path, "integrated repair final", path_order="lexical")
    previous_map = {rel: digest for digest, rel in previous_rows}
    base_map = {rel: digest for digest, rel in base_rows}
    final_map = {rel: digest for digest, rel in final_rows}
    expected_base = dict(previous_map)
    for owner in owners:
        if owner["path"] in expected_base:
            raise VerificationError(f"integrated repair extra owner is already in the preceding source set: {owner['path']}")
        expected_base[owner["path"]] = owner["sha256"]
    if base_map != expected_base or len(base_rows) != base["count"]:
        raise VerificationError("integrated repair paired base differs from the stage-twelve final plus the baseline extra owners")
    if set(final_map) != set(base_map) or len(final_rows) != stage["count"]:
        raise VerificationError("integrated repair final changed the paired base path set")
    changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
    if changed != set(source_paths):
        raise VerificationError("integrated repair final must change exactly the pinned source owners")
    for owner in owners:
        if final_map.get(owner["path"]) != owner["final_sha256"]:
            raise VerificationError(f"integrated repair final hash differs from the pinned extra owner: {owner['path']}")
    if any(net.get(rel) != "modified" for rel in source_paths):
        raise VerificationError("integrated repair source owners must all be modified net owners")
    return {
        "base_count": len(base_map),
        "final_count": len(final_map),
        "changed_source_owners": sorted(changed),
        "unchanged_source_count": len(base_map) - len(changed),
        "paired_after": previous["name"],
        "extra_owner_count": extra_count,
    }


def _verify_integrated_repair_v2_inputs(
    payload: Path, baseline: Path, stage_by_name: dict[str, dict[str, Any]], net: dict[str, str]
) -> dict[str, Any]:
    """Pin stage fourteen to stage thirteen plus baseline extra owners; the final adds new files."""
    base = EXPECTED.get("integrated_repair_v2_base")
    previous = stage_by_name.get("integrated-repair-v1")
    stage = stage_by_name.get("integrated-repair-v2")
    if not isinstance(base, dict) or previous is None or stage is None:
        raise VerificationError("integrated repair v2 paired base or stage is missing")
    order = EXPECTED["patch_order"]
    index = order.index("integrated-repair-v2")
    if base.get("paired_after") != previous["name"] or index == 0 or order[index - 1] != "integrated-repair-v1":
        raise VerificationError("integrated repair v2 paired base must immediately follow integrated-repair-v1")
    if base.get("path_order") != "lexical" or previous.get("path_order") != "lexical" or stage.get("path_order") != "lexical":
        raise VerificationError("integrated repair v2 base, predecessor and final must retain lexical path ordering")
    owners = base.get("extra_owners")
    added = base.get("added_paths")
    source_paths = EXPECTED.get("integrated_repair_v2_source_paths")
    if not isinstance(owners, list) or not isinstance(added, list) or not isinstance(source_paths, list):
        raise VerificationError("integrated repair v2 pins are malformed")
    owner_paths = [owner.get("path") for owner in owners]
    base_extra = list(previous.get("extra_paths", [])) + owner_paths
    if previous.get("extra_paths") != INTEGRATED_REPAIR_EXTRA_PATHS or stage.get("extra_paths") != base_extra + added:
        raise VerificationError("integrated repair v2 stage has an unexpected stage-scoped path set")
    if sorted(set(source_paths)) != sorted(source_paths) or sorted(set(added)) != sorted(added):
        raise VerificationError("integrated repair v2 source or added path pin is not unique")
    if base.get("count") != previous["count"] + len(owners) or stage.get("count") != base["count"] + len(added):
        raise VerificationError("integrated repair v2 counts must be predecessor plus baseline extras (base) plus added files (final)")
    if base.get("file") != "integrated-repair-v2-base.sha256" or stage.get("file") != "integrated-repair-v2.sha256":
        raise VerificationError("integrated repair v2 paired input filenames differ from the pinned contract")
    for descriptor in (base, stage):
        if descriptor.get("sha256") != EXPECTED["artifacts"].get(descriptor["file"]):
            raise VerificationError(f"integrated repair v2 manifest pin is stale for {descriptor['file']}")
    for owner in owners:
        owner_path = _path_without_symlinks(baseline, owner["path"], "integrated repair v2 baseline extra owner")
        if not owner_path.is_file() or owner_path.is_symlink():
            raise VerificationError(f"integrated repair v2 baseline extra owner is missing or not a regular file: {owner['path']}")
        owner_bytes = owner_path.read_bytes()
        if _sha(owner_bytes) != owner["sha256"] or _git_blob_sha1(owner_bytes) != owner["git_blob_sha1"]:
            raise VerificationError(f"baseline extra owner SHA-256 or Git blob identity mismatch: {owner['path']}")
    for rel in added:
        if (baseline / rel).exists() or (baseline / rel).is_symlink():
            raise VerificationError(f"integrated repair v2 added file must not exist in the baseline: {rel}")
    base_path = _path_without_symlinks(payload, base["file"], "integrated repair v2 paired base")
    previous_path = _path_without_symlinks(payload, previous["file"], "integrated repair v2 predecessor final")
    final_path = _path_without_symlinks(payload, stage["file"], "integrated repair v2 final")
    previous_map = {rel: digest for digest, rel in _parse_sha_manifest(previous_path, "integrated repair v2 predecessor final", path_order="lexical")}
    base_rows = _parse_sha_manifest(base_path, "integrated repair v2 paired base", path_order="lexical")
    final_rows = _parse_sha_manifest(final_path, "integrated repair v2 final", path_order="lexical")
    base_map = {rel: digest for digest, rel in base_rows}
    final_map = {rel: digest for digest, rel in final_rows}
    expected_base = dict(previous_map)
    for owner in owners:
        if owner["path"] in expected_base:
            raise VerificationError(f"integrated repair v2 extra owner is already in the preceding source set: {owner['path']}")
        expected_base[owner["path"]] = owner["sha256"]
    if base_map != expected_base or len(base_rows) != base["count"]:
        raise VerificationError("integrated repair v2 paired base differs from the stage-thirteen final plus the baseline extra owners")
    if set(final_map) != set(base_map) | set(added) or len(final_rows) != stage["count"]:
        raise VerificationError("integrated repair v2 final must equal the paired base path set plus exactly the added files")
    changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
    if changed != set(source_paths):
        raise VerificationError("integrated repair v2 final must change exactly the pinned source owners")
    for owner in owners:
        if final_map.get(owner["path"]) != owner["final_sha256"]:
            raise VerificationError(f"integrated repair v2 final hash differs from the pinned extra owner: {owner['path']}")
    if any(net.get(rel) not in ("modified", "new") for rel in source_paths):
        raise VerificationError("integrated repair v2 source owners must all be net owners")
    if any(net.get(rel) != "new" for rel in added):
        raise VerificationError("integrated repair v2 added files must all be new net owners")
    return {
        "base_count": len(base_map),
        "final_count": len(final_map),
        "changed_source_owners": sorted(changed),
        "added_files": sorted(added),
        "paired_after": previous["name"],
        "extra_owner_count": len(owners),
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

    expected_integrated_repair_inputs = {"integrated-repair-v1.patch", "integrated-repair-v1-base.sha256", "integrated-repair-v1.sha256"}
    actual_integrated_repair_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("integrated-repair-v1")}
    if actual_integrated_repair_inputs - expected_integrated_repair_inputs:
        unexpected = sorted(actual_integrated_repair_inputs - expected_integrated_repair_inputs)
        raise VerificationError(f"unexpected integrated repair input: {unexpected[0]}")
    expected_integrated_repair_v2_inputs = {"integrated-repair-v2.patch", "integrated-repair-v2-base.sha256", "integrated-repair-v2.sha256"}
    actual_integrated_repair_v2_inputs = {entry.name for entry in payload.iterdir() if entry.name.startswith("integrated-repair-v2")}
    if actual_integrated_repair_v2_inputs - expected_integrated_repair_v2_inputs:
        unexpected = sorted(actual_integrated_repair_v2_inputs - expected_integrated_repair_v2_inputs)
        raise VerificationError(f"unexpected integrated repair v2 input: {unexpected[0]}")

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
    lexical_stage_names = {"compile-repairs-v1", "cumulative-hir-v1", "next-hir-v1", "forced-root-wire-v1", "dependency-compile-v1", "std-compile-v1", "quickjs-runtime-compile-v1", "quickjs-api-followup-v1", "integrated-repair-v1", "integrated-repair-v2"}
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
        "integrated-repair-v1": INTEGRATED_REPAIR_EXTRA_PATHS,
        "integrated-repair-v2": INTEGRATED_REPAIR_V2_FINAL_EXTRA_PATHS,
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
    integrated_repair_inputs = _verify_integrated_repair_inputs(payload, baseline, stage_by_name, net)
    integrated_repair_v2_inputs = _verify_integrated_repair_v2_inputs(payload, baseline, stage_by_name, net)
    return {
        "baseline": identity,
        "inputs": {**EXPECTED["payload_patches"], **EXPECTED["artifacts"]},
        "native_owner_count": len(owner_map),
        "net_owner_count": len(net),
        "dependency_compile": dependency_inputs,
        "std_compile": std_compile_inputs,
        "quickjs_runtime_compile": quickjs_runtime_inputs,
        "quickjs_api_followup": quickjs_api_followup_inputs,
        "integrated_repair": integrated_repair_inputs,
        "integrated_repair_v2": integrated_repair_v2_inputs,
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
        "integrated-repair-v1": "integrated-repair-v1.patch",
        "integrated-repair-v2": "integrated-repair-v2.patch",
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
            elif stage["name"] == "integrated-repair-v1":
                repair_base = EXPECTED["integrated_repair_base"]
                paired_stage_name = repair_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("integrated repair paired base is not immediately after the QuickJS API follow-up final")
                previous = stage_by_name[paired_stage_name]
                previous_result = verify_stage(
                    candidate,
                    payload / previous["file"],
                    previous["count"],
                    previous.get("extra_paths", []),
                    previous.get("path_order", "components"),
                )
                if previous_result["manifest_sha256"] != state["stages"][-1]["manifest_sha256"]:
                    raise VerificationError("integrated repair predecessor no longer matches the reconstructed QuickJS API follow-up final")
                paired = verify_stage(
                    candidate,
                    payload / repair_base["file"],
                    repair_base["count"],
                    INTEGRATED_REPAIR_EXTRA_PATHS,
                    repair_base["path_order"],
                )
                state["paired_integrated_repair_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_owner_count": len(repair_base["extra_owners"]),
                }
            elif stage["name"] == "integrated-repair-v2":
                repair_base = EXPECTED["integrated_repair_v2_base"]
                paired_stage_name = repair_base["paired_after"]
                if not state["stages"] or state["stages"][-1]["stage"] != paired_stage_name:
                    raise VerificationError("integrated repair v2 paired base is not immediately after the integrated repair final")
                previous = stage_by_name[paired_stage_name]
                previous_result = verify_stage(
                    candidate,
                    payload / previous["file"],
                    previous["count"],
                    previous.get("extra_paths", []),
                    previous.get("path_order", "components"),
                )
                if previous_result["manifest_sha256"] != state["stages"][-1]["manifest_sha256"]:
                    raise VerificationError("integrated repair v2 predecessor no longer matches the reconstructed integrated repair final")
                paired = verify_stage(
                    candidate,
                    payload / repair_base["file"],
                    repair_base["count"],
                    INTEGRATED_REPAIR_V2_BASE_EXTRA_PATHS,
                    repair_base["path_order"],
                )
                state["paired_integrated_repair_v2_base"] = {
                    "count": paired["count"],
                    "manifest_sha256": paired["manifest_sha256"],
                    "paired_after": paired_stage_name,
                    "extra_owner_count": len(repair_base["extra_owners"]),
                    "added_file_count": len(repair_base["added_paths"]),
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
