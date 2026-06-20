use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::error::AppError;

pub(super) struct CliArgs {
    values: VecDeque<OsString>,
}

impl CliArgs {
    pub(super) fn new(values: impl IntoIterator<Item = OsString>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(super) fn peek_is(&self, expected: &str) -> bool {
        matches!(self.values.front().and_then(|value| value.to_str()), Some(value) if value == expected)
    }

    pub(super) fn pop_os(&mut self, label: &str) -> Result<OsString, AppError> {
        self.values
            .pop_front()
            .ok_or_else(|| AppError::user(format!("{label} 값이 필요합니다.")))
    }

    pub(super) fn pop_string(&mut self, label: &str) -> Result<String, AppError> {
        os_string_to_string(self.pop_os(label)?, label)
    }

    pub(super) fn pop_optional_string(&mut self) -> Result<Option<String>, AppError> {
        self.values
            .pop_front()
            .map(|value| os_string_to_string(value, "argument"))
            .transpose()
    }

    pub(super) fn pop_path(&mut self, label: &str) -> Result<PathBuf, AppError> {
        Ok(PathBuf::from(self.pop_os(label)?))
    }

    pub(super) fn pop_node_id(&mut self, label: &str) -> Result<i64, AppError> {
        let value = self.pop_string(label)?;
        parse_node_id_text(&value, label)
    }

    pub(super) fn ensure_empty(self, command: &str) -> Result<(), AppError> {
        if self.values.is_empty() {
            return Ok(());
        }

        let extra = self
            .values
            .front()
            .and_then(|value| value.to_str())
            .unwrap_or("<non-unicode>");
        Err(AppError::user(format!(
            "{command} 명령에서 알 수 없는 인수입니다: {extra}"
        )))
    }
}

pub(super) fn set_once<T>(target: &mut Option<T>, value: T, option: &str) -> Result<(), AppError> {
    if target.is_some() {
        return Err(AppError::user(format!(
            "{option} 옵션은 한 번만 사용할 수 있습니다."
        )));
    }

    *target = Some(value);
    Ok(())
}

pub(super) fn parse_node_id_text(value: &str, label: &str) -> Result<i64, AppError> {
    let node_id = value
        .parse::<i64>()
        .map_err(|_| AppError::user(format!("{label} 값은 1 이상의 정수여야 합니다: {value}")))?;
    if node_id <= 0 {
        return Err(AppError::user(format!(
            "{label} 값은 1 이상의 정수여야 합니다: {value}"
        )));
    }

    Ok(node_id)
}

fn os_string_to_string(value: OsString, label: &str) -> Result<String, AppError> {
    value
        .into_string()
        .map_err(|_| AppError::user(format!("{label} 값은 유효한 Unicode 문자열이어야 합니다.")))
}
