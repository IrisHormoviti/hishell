use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub enum ConfigValue {
	String(String),
	Number(f64),
	Boolean(bool),
	Array(Vec<ConfigValue>),
	Dictionary(HashMap<String, ConfigValue>),
	Vector2(f64, f64),
}

impl ConfigValue {
	pub fn to_json_string(&self) -> String {
		match self {
			ConfigValue::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
			ConfigValue::Number(n) => n.to_string(),
			ConfigValue::Boolean(b) => {
				if *b {
					"true".to_string()
				} else {
					"false".to_string()
				}
			}
			ConfigValue::Array(arr) => {
				let items: Vec<String> = arr.iter().map(|v| v.to_json_string()).collect();
				format!("[{}]", items.join(","))
			}
			ConfigValue::Dictionary(dict) => {
				let items: Vec<String> = dict
					.iter()
					.map(|(k, v)| format!("\"{}\":{}", k.replace('"', "\\\""), v.to_json_string()))
					.collect();
				format!("{{{}}}", items.join(","))
			}
			ConfigValue::Vector2(x, y) => {
				// Represent Vector2 as an object {x: ..., y: ...}
				format!("{{\"x\":{},\"y\":{}}}", x, y)
			}
		}
	}
}

#[derive(Debug, Clone)]
pub struct ConfigError {
	pub file: Option<String>,
	pub message: String,
}

#[derive(Clone)]
pub struct ParseOutput {
	pub sections: HashMap<String, HashMap<String, ConfigValue>>,
	pub origins: HashMap<(String, String), String>,
	pub errors: Vec<ConfigError>,
}

pub struct ConfigParser;

impl ConfigParser {
	pub fn parse_files(paths: &[&Path]) -> ParseOutput {
		let mut sections: HashMap<String, HashMap<String, ConfigValue>> = HashMap::new();
		let mut origins: HashMap<(String, String), String> = HashMap::new();
		let mut errors = Vec::new();

		for path in paths {
			let path_str = path.to_string_lossy().to_string();
			let (parsed, file_errors) = Self::parse_file(path);

			for (section, items) in parsed {
				for (key, value) in items {
					origins.insert((section.clone(), key.clone()), path_str.clone());
					sections
						.entry(section.clone())
						.or_insert_with(HashMap::new)
						.insert(key, value);
				}
			}

			errors.extend(file_errors);
		}

		ParseOutput {
			sections,
			origins,
			errors,
		}
	}

	pub fn parse_file(
		path: &Path,
	) -> (
		HashMap<String, HashMap<String, ConfigValue>>,
		Vec<ConfigError>,
	) {
		let path_str = path.to_string_lossy().to_string();

		match fs::read_to_string(path) {
			Ok(content) => Self::parse_str(&content, &path_str),
			Err(err) => {
				// A missing config file is expected; only report files that
				// exist but could not be read.
				let mut errors = Vec::new();
				if path.exists() {
					errors.push(ConfigError {
						file: Some(path_str.clone()),
						message: format!("{}: {}", path_str, err),
					});
				}
				(HashMap::new(), errors)
			}
		}
	}

	/// Parses config content that does not come from a file, such as the desktop
	/// entry embedded in an AppImage.
	pub fn parse_str(
		content: &str,
		source: &str,
	) -> (
		HashMap<String, HashMap<String, ConfigValue>>,
		Vec<ConfigError>,
	) {
		let mut config = HashMap::new();
		let mut errors = Vec::new();
		let mut current_section: Option<String> = None;
		let mut section_map = HashMap::new();

		for (index, raw) in content.lines().enumerate() {
			let line = raw.trim();
			if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
				continue;
			}

			if line.starts_with('[') {
				if line.ends_with(']') {
					if let Some(section) = current_section.take() {
						config.insert(section, std::mem::take(&mut section_map));
					}
					current_section = Some(line[1..line.len() - 1].to_string());
				} else {
					errors.push(ConfigError {
						file: Some(source.to_string()),
						message: format!(
							"{}:{}: malformed section header: {}",
							source,
							index + 1,
							line
						),
					});
				}
			} else if let Some(idx) = line.find('=') {
				if current_section.is_some() {
					let key = line[..idx].trim().to_string();
					let val_str = line[idx + 1..].trim();
					let value =
						if current_section.as_deref() == Some("Desktop Entry") && key == "Exec" {
							ConfigValue::String(if val_str == "\"\"" {
								String::new()
							} else {
								val_str.to_string()
							})
						} else {
							Self::parse_value(val_str)
						};
					section_map.insert(key, value);
				} else {
					errors.push(ConfigError {
						file: Some(source.to_string()),
						message: format!(
							"{}:{}: '{}' is not inside a [section]",
							source,
							index + 1,
							line
						),
					});
				}
			} else {
				errors.push(ConfigError {
					file: Some(source.to_string()),
					message: format!("{}:{}: malformed line: {}", source, index + 1, line),
				});
			}
		}

		if let Some(section) = current_section.take() {
			config.insert(section, section_map);
		}

		(config, errors)
	}

	pub fn set_value(path: &Path, section: &str, key: &str, value: &str) {
		let content = fs::read_to_string(path).unwrap_or_default();
		let mut out = String::new();
		let mut in_section = false;
		let mut replaced = false;

		for line in content.lines() {
			let trimmed = line.trim();

			if trimmed.starts_with('[') && trimmed.ends_with(']') {
				if in_section && !replaced {
					out.push_str(&format!("{} = {}\n", key, value));
					replaced = true;
				}
				in_section = trimmed == format!("[{}]", section);
			} else if in_section && trimmed.starts_with(key) {
				if let Some((k, _)) = trimmed.split_once('=') {
					if k.trim() == key {
						out.push_str(&format!("{} = {}\n", key, value));
						replaced = true;
						continue;
					}
				}
			}

			out.push_str(line);
			out.push('\n');
		}

		if !replaced {
			if !in_section {
				out.push_str(&format!("\n[{}]\n", section));
			}
			out.push_str(&format!("{} = {}\n", key, value));
		}

		if let Some(parent) = path.parent() {
			let _ = fs::create_dir_all(parent);
		}

		if let Err(e) = fs::write(path, out) {
			eprintln!("Failed to save config: {}", e);
		}
	}

	fn parse_value(s: &str) -> ConfigValue {
		let s = s.trim();
		if s == "true" {
			ConfigValue::Boolean(true)
		} else if s == "false" {
			ConfigValue::Boolean(false)
		} else if s.starts_with('"') && s.ends_with('"') {
			ConfigValue::String(s[1..s.len() - 1].to_string())
		} else if s.starts_with('[') && s.ends_with(']') {
			let inner = &s[1..s.len() - 1];
			// Simple split by comma for demo purposes.
			// Real parser would handle nested structures.
			let items = Self::split_comma(inner);
			ConfigValue::Array(items.into_iter().map(|i| Self::parse_value(&i)).collect())
		} else if s.starts_with('{') && s.ends_with('}') {
			let inner = &s[1..s.len() - 1];
			let items = Self::split_comma(inner);
			let mut dict = HashMap::new();
			for item in items {
				// handle either "key": value or key=value
				if let Some(idx) =
					Self::find_unquoted(&item, ':').or_else(|| Self::find_unquoted(&item, '='))
				{
					let k = item[..idx].trim();
					let v = item[idx + 1..].trim();
					let key_str = if k.starts_with('"') && k.ends_with('"') {
						k[1..k.len() - 1].to_string()
					} else {
						k.to_string()
					};
					dict.insert(key_str, Self::parse_value(v));
				}
			}
			ConfigValue::Dictionary(dict)
		} else if s.starts_with('(') && s.ends_with(')') {
			let inner = &s[1..s.len() - 1];
			let parts: Vec<&str> = inner.split(',').collect();
			if parts.len() == 2 {
				let x = parts[0].trim().parse::<f64>().unwrap_or(0.0);
				let y = parts[1].trim().parse::<f64>().unwrap_or(0.0);
				ConfigValue::Vector2(x, y)
			} else {
				ConfigValue::String(s.to_string())
			}
		} else if let Ok(n) = s.parse::<f64>() {
			ConfigValue::Number(n)
		} else {
			ConfigValue::String(s.to_string())
		}
	}

	/// First index of `target` that is not inside a quoted string.
	fn find_unquoted(s: &str, target: char) -> Option<usize> {
		let mut in_string = false;
		for (i, c) in s.char_indices() {
			match c {
				'"' => in_string = !in_string,
				c if c == target && !in_string => return Some(i),
				_ => {}
			}
		}
		None
	}

	fn split_comma(s: &str) -> Vec<String> {
		let mut result = Vec::new();
		let mut current = String::new();
		let mut depth_array = 0;
		let mut depth_dict = 0;
		let mut depth_vec = 0;
		let mut in_string = false;

		for c in s.chars() {
			match c {
				'"' => in_string = !in_string,
				'[' if !in_string => depth_array += 1,
				']' if !in_string => depth_array -= 1,
				'{' if !in_string => depth_dict += 1,
				'}' if !in_string => depth_dict -= 1,
				'(' if !in_string => depth_vec += 1,
				')' if !in_string => depth_vec -= 1,
				',' if !in_string && depth_array == 0 && depth_dict == 0 && depth_vec == 0 => {
					result.push(current.trim().to_string());
					current.clear();
					continue;
				}
				_ => {}
			}
			current.push(c);
		}
		if !current.trim().is_empty() {
			result.push(current.trim().to_string());
		}
		result
	}
}
