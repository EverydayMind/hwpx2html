//! Private, single-worker ABI. Every allocation is a boxed byte slice;
//! lengths are exact, and conversion consumes the input allocation once.
//! JS copies the result out before freeing it and terminating the worker.
use hwpx2html::convert::{convert_bytes, ConvertOptions, ConvertOutcome, Summary};
use hwpx2html::diagnostic::Diagnostic;
use hwpx2html::error::ConvertError;
use hwpx2html::render::RenderOptions;
use serde::Serialize;

pub struct Output {
    metadata: Vec<u8>,
    html: Vec<u8>,
}

#[derive(Serialize)]
struct Metadata {
    status: &'static str,
    diagnostics: Vec<Diagnostic>,
    summary: Option<Summary>,
    unsupported_objects: usize,
    error: Option<ErrorMessage>,
}

#[derive(Serialize)]
struct ErrorMessage {
    code: &'static str,
    message: &'static str,
    detail: String,
}

fn error_message(error: &ConvertError) -> ErrorMessage {
    let (code, message) = match error {
        ConvertError::UnsupportedFormat(_) => ("unsupported_format", "ZIP 기반 HWPX 파일이 아닙니다. 한글에서 HWPX 형식으로 다시 저장해 주세요."),
        ConvertError::EncryptedOrProtected(_) => ("encrypted_document", "암호화되거나 보호된 문서는 변환할 수 없습니다. 한글에서 보호를 해제한 사본을 저장해 주세요."),
        ConvertError::InputTooLarge { .. } => ("input_too_large", "파일 크기가 64MiB 한도를 넘었습니다. 문서를 나누어 저장해 주세요."),
        ConvertError::MemoryBudgetExceeded { .. } => ("memory_budget_exceeded", "문서가 기본 메모리 예산을 넘었습니다. 문서를 나누거나 CLI의 메모리 예산 옵션을 사용해 주세요."),
        ConvertError::TooManyEntries { .. } | ConvertError::EntryTooLarge { .. } | ConvertError::UnpackedTooLarge { .. } => ("package_limit", "압축 해제 크기 또는 파일 수가 안전 한도를 넘었습니다. 문서를 나누어 저장해 주세요."),
        ConvertError::UnsafeZipPath(_) | ConvertError::DuplicateZipPath(_) => ("unsafe_package", "HWPX 내부 파일 경로가 올바르지 않습니다. 한글에서 다시 저장해 주세요."),
        ConvertError::MissingEntry(_) | ConvertError::UnsupportedSchema(_) => ("unsupported_schema", "필수 문서 정보가 없거나 지원하지 않는 HWPX 구조입니다. 한글에서 다시 저장해 주세요."),
        ConvertError::Xml { .. } | ConvertError::InvalidValue { .. } | ConvertError::Zip(_) => ("invalid_document", "HWPX 문서가 손상되었거나 읽을 수 없는 값이 있습니다. 한글에서 열어 다시 저장해 주세요."),
        ConvertError::StrictUnsupported => ("strict_rejected", "엄격 모드에서 지원하지 않는 내용을 발견해 결과를 만들지 않았습니다. 경고를 확인해 주세요."),
        ConvertError::UnwritableContent(_) => ("unwritable_content", "현재 출력기가 처리할 수 없는 문서 구조입니다. 아래 원문 사유를 확인해 주세요."),
        _ => ("conversion_failed", "문서를 변환하지 못했습니다. 아래 원문 사유를 확인해 주세요."),
    };
    ErrorMessage {
        code,
        message,
        detail: error.to_string(),
    }
}

fn output(name: &str, bytes: Vec<u8>, flags: u32) -> Output {
    let options = ConvertOptions {
        render: RenderOptions {
            page_navigation: flags & 1 != 0,
            infer_structure: flags & 2 != 0,
            reading_view: flags & 8 != 0,
            ..RenderOptions::default()
        },
        strict: flags & 4 != 0,
        ..ConvertOptions::default()
    };
    let (metadata, html) = match convert_bytes(name, bytes, &options) {
        Ok(ConvertOutcome::Converted(conversion)) => {
            let html = conversion.bundle.to_single_html().into_bytes();
            (
                Metadata {
                    status: "converted",
                    diagnostics: conversion.diagnostics,
                    unsupported_objects: conversion.summary.unsupported_objects,
                    summary: Some(conversion.summary),
                    error: None,
                },
                html,
            )
        }
        Ok(ConvertOutcome::Rejected {
            diagnostics,
            unsupported_objects,
        }) => (
            Metadata {
                status: "rejected",
                diagnostics,
                summary: None,
                unsupported_objects,
                error: Some(error_message(&ConvertError::StrictUnsupported)),
            },
            Vec::new(),
        ),
        Err(error) => (
            Metadata {
                status: "failed",
                diagnostics: Vec::new(),
                summary: None,
                unsupported_objects: 0,
                error: Some(error_message(&error)),
            },
            Vec::new(),
        ),
    };
    Output {
        metadata: serde_json::to_vec(&metadata).expect("serializable metadata"),
        html,
    }
}

#[no_mangle]
pub extern "C" fn hp_alloc(length: usize) -> *mut u8 {
    Box::into_raw(vec![0_u8; length].into_boxed_slice()) as *mut u8
}

/// # Safety
/// `pointer` must be the live allocation from hp_alloc(length), exactly once.
#[no_mangle]
pub unsafe extern "C" fn hp_free(pointer: *mut u8, length: usize) {
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
        pointer, length,
    )));
}

/// # Safety
/// Name is a live UTF-8 allocation; input is an exclusive hp_alloc allocation
/// of input_length bytes. This consumes input even when conversion fails.
#[no_mangle]
pub unsafe extern "C" fn hp_convert(
    name_pointer: *const u8,
    name_length: usize,
    input_pointer: *mut u8,
    input_length: usize,
    flags: u32,
) -> *mut Output {
    let bytes = Box::from_raw(std::ptr::slice_from_raw_parts_mut(
        input_pointer,
        input_length,
    ))
    .into_vec();
    let name = String::from_utf8_lossy(std::slice::from_raw_parts(name_pointer, name_length));
    Box::into_raw(Box::new(output(&name, bytes, flags)))
}

/// # Safety
/// Handle must be a live hp_convert result, until hp_result_free is called.
#[no_mangle]
pub unsafe extern "C" fn hp_metadata_pointer(handle: *const Output) -> *const u8 {
    (*handle).metadata.as_ptr()
}

/// # Safety
/// Handle must be a live hp_convert result.
#[no_mangle]
pub unsafe extern "C" fn hp_metadata_length(handle: *const Output) -> usize {
    (*handle).metadata.len()
}

/// # Safety
/// Handle must be a live hp_convert result.
#[no_mangle]
pub unsafe extern "C" fn hp_html_pointer(handle: *const Output) -> *const u8 {
    (*handle).html.as_ptr()
}

/// # Safety
/// Handle must be a live hp_convert result.
#[no_mangle]
pub unsafe extern "C" fn hp_html_length(handle: *const Output) -> usize {
    (*handle).html.len()
}

/// # Safety
/// Handle must be a live hp_convert result, freed exactly once.
#[no_mangle]
pub unsafe extern "C" fn hp_result_free(handle: *mut Output) {
    drop(Box::from_raw(handle));
}
