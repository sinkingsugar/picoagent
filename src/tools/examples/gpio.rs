//! GPIO tool — digital read/write on ESP32 pins.
//!
//! Example of a tool that directly controls hardware.

use crate::tools::{Tool, ToolOutput};
use anyhow::{bail, Result};
use esp_idf_svc::hal::gpio::{AnyIOPin, Input, Output, PinDriver, Pull};
use serde_json::Value;
use std::collections::BTreeMap;

enum GpioPin<'a> {
    Output(PinDriver<'a, AnyIOPin, Output>),
    Input(PinDriver<'a, AnyIOPin, Input>),
}

/// GPIO tool that manages a set of named pins.
///
/// ```rust
/// let mut gpio = GpioTool::new();
/// gpio.add_output("led", peripherals.pins.gpio2.into())?;
/// gpio.add_input("button", peripherals.pins.gpio0.into())?;
/// tools.register(gpio);
/// ```
pub struct GpioTool<'a> {
    pins: BTreeMap<String, GpioPin<'a>>,
}

impl<'a> GpioTool<'a> {
    pub fn new() -> Self {
        Self {
            pins: BTreeMap::new(),
        }
    }

    pub fn add_output(&mut self, name: impl Into<String>, pin: AnyIOPin) -> Result<()> {
        let driver = PinDriver::output(pin)?;
        self.pins.insert(name.into(), GpioPin::Output(driver));
        Ok(())
    }

    pub fn add_input(&mut self, name: impl Into<String>, pin: AnyIOPin) -> Result<()> {
        let mut driver = PinDriver::input(pin)?;
        driver.set_pull(Pull::Up)?;
        self.pins.insert(name.into(), GpioPin::Input(driver));
        Ok(())
    }
}

impl<'a> Tool for GpioTool<'a> {
    fn name(&self) -> &'static str {
        "gpio"
    }

    fn description(&self) -> &'static str {
        "Control GPIO pins. Actions: 'set' (output high/low), 'read' (get state), 'list' (show all)."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "read", "list"],
                    "description": "The action to perform"
                },
                "pin": {
                    "type": "string",
                    "description": "Pin name (e.g. 'led', 'relay1')"
                },
                "value": {
                    "type": "string",
                    "enum": ["high", "low"],
                    "description": "Pin value for 'set' action"
                }
            },
            "required": ["action"]
        })
    }

    fn execute(&mut self, params: Value) -> Result<ToolOutput> {
        let action = params["action"].as_str().unwrap_or("list");

        match action {
            "list" => {
                let listing: Vec<String> = self
                    .pins
                    .iter()
                    .map(|(name, pin)| {
                        let dir = match pin {
                            GpioPin::Output(_) => "output",
                            GpioPin::Input(_) => "input",
                        };
                        format!("  {name}: {dir}")
                    })
                    .collect();

                if listing.is_empty() {
                    Ok(ToolOutput::ok("No GPIO pins configured."))
                } else {
                    Ok(ToolOutput::ok(format!("Pins:\n{}", listing.join("\n"))))
                }
            }
            "read" => {
                let pin_name = params["pin"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'pin' required"))?;

                let pin = self
                    .pins
                    .get(pin_name)
                    .ok_or_else(|| anyhow::anyhow!("unknown pin: {pin_name}"))?;

                let level = match pin {
                    GpioPin::Input(d) => if d.is_high() { "high" } else { "low" },
                    GpioPin::Output(d) => if d.is_set_high() { "high" } else { "low" },
                };

                Ok(ToolOutput::ok(format!("{pin_name}: {level}")))
            }
            "set" => {
                let pin_name = params["pin"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'pin' required"))?;
                let value = params["value"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("'value' required"))?;

                let pin = self
                    .pins
                    .get_mut(pin_name)
                    .ok_or_else(|| anyhow::anyhow!("unknown pin: {pin_name}"))?;

                match pin {
                    GpioPin::Output(d) => {
                        match value {
                            "high" => d.set_high()?,
                            "low" => d.set_low()?,
                            _ => bail!("value must be 'high' or 'low'"),
                        }
                        Ok(ToolOutput::ok(format!("{pin_name} -> {value}")))
                    }
                    GpioPin::Input(_) => {
                        Ok(ToolOutput::err(format!("{pin_name} is input, cannot set")))
                    }
                }
            }
            _ => Ok(ToolOutput::err(format!("unknown action: {action}"))),
        }
    }
}
