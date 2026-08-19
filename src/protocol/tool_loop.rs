#![allow(dead_code)] // Task 12 connects the completed state machine to the runner.

use serde_json::json;

use crate::evidence::ToolLoopOutcome;
use crate::protocol::stream::{AssistantTurn, ToolCall};
use crate::protocol::tools::ExecutedToolResult;

pub(crate) const MAX_ASSISTANT_TURNS: usize = 4;

#[derive(Debug)]
pub(crate) struct ToolLoopState {
    check_id: &'static str,
    assistant_turns: usize,
    weather_attempts: usize,
    terminal: Option<LoopDecision>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LoopDecision {
    Complete,
    Continue(Vec<ExecutedToolResult>),
    Stop {
        outcome: ToolLoopOutcome,
        contract_errors: Vec<String>,
    },
}

impl ToolLoopState {
    pub(crate) fn new(check_id: &'static str) -> Option<Self> {
        matches!(check_id, "046" | "047" | "048" | "049").then_some(Self {
            check_id,
            assistant_turns: 0,
            weather_attempts: 0,
            terminal: None,
        })
    }

    pub(crate) fn advance(&mut self, turn: &AssistantTurn) -> LoopDecision {
        if let Some(decision) = &self.terminal {
            return decision.clone();
        }

        self.assistant_turns += 1;

        if turn.tool_calls.is_empty() {
            return self.finish(LoopDecision::Complete);
        }
        if self.assistant_turns >= MAX_ASSISTANT_TURNS {
            return self.finish(stop(ToolLoopOutcome::MaxTurnsExceeded, Vec::new()));
        }

        let mut contract_errors = Vec::new();
        if self.check_id == "049" && turn.tool_calls.len() > 1 {
            contract_errors.push("tool_loop.multiple_tool_calls:/tool_calls".into());
        }
        contract_errors.extend(
            turn.tool_calls
                .iter()
                .enumerate()
                .filter_map(|(position, call)| self.validate_call(position, call)),
        );
        if !contract_errors.is_empty() {
            return self.finish(stop(ToolLoopOutcome::InvalidTurn, contract_errors));
        }

        let results = turn
            .tool_calls
            .iter()
            .cloned()
            .map(|call| self.execute(call))
            .collect();
        LoopDecision::Continue(results)
    }

    fn finish(&mut self, decision: LoopDecision) -> LoopDecision {
        self.terminal = Some(decision.clone());
        decision
    }

    fn validate_call(&self, position: usize, call: &ToolCall) -> Option<String> {
        let name = logical_tool_name(self.check_id, &call.name);
        let allowed = match name {
            "get_weather" => true,
            "get_time" => self.check_id == "047",
            _ => {
                return Some(format!(
                    "tool_loop.unknown_tool:/tool_calls/{position}/name"
                ));
            }
        };
        if !allowed {
            return Some(format!(
                "tool_loop.tool_not_allowed:/tool_calls/{position}/name"
            ));
        }

        let valid_arguments = match name {
            "get_weather" => call.arguments == json!({"city": "Beijing"}),
            "get_time" => call.arguments == json!({"zone": "UTC"}),
            _ => unreachable!("unknown tools return before argument validation"),
        };
        (!valid_arguments)
            .then(|| format!("tool_loop.invalid_arguments:/tool_calls/{position}/arguments"))
    }

    fn execute(&mut self, call: ToolCall) -> ExecutedToolResult {
        let name = logical_tool_name(self.check_id, &call.name);
        let (output, is_error) = match name {
            "get_weather" if self.check_id == "049" && self.weather_attempts == 0 => {
                ("ERROR: timeout", true)
            }
            "get_weather" => ("WEATHER_SUNNY", false),
            "get_time" => ("TIME_UTC_12:00", false),
            _ => unreachable!("calls are allowlisted before execution"),
        };
        if name == "get_weather" {
            self.weather_attempts += 1;
        }
        ExecutedToolResult {
            call,
            output: output.into(),
            is_error,
        }
    }
}

fn logical_tool_name<'a>(check_id: &str, name: &'a str) -> &'a str {
    if check_id == "047" && name == "doctor__get_weather" {
        "get_weather"
    } else {
        name
    }
}

fn stop(outcome: ToolLoopOutcome, contract_errors: Vec<String>) -> LoopDecision {
    LoopDecision::Stop {
        outcome,
        contract_errors,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::evidence::ToolLoopOutcome;
    use crate::protocol::stream::{AssistantTurn, ProtocolHistory, ToolCall, ToolCorrelation};

    use super::{LoopDecision, ToolLoopState};

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        custom_call(
            0,
            ToolCorrelation::Required("call-0".into()),
            name,
            arguments,
        )
    }

    fn custom_call(
        index: usize,
        correlation: ToolCorrelation,
        name: &str,
        arguments: serde_json::Value,
    ) -> ToolCall {
        ToolCall {
            index,
            correlation,
            name: name.into(),
            arguments,
        }
    }

    fn turn(tool_calls: Vec<ToolCall>, final_text: &str) -> AssistantTurn {
        AssistantTurn {
            history: ProtocolHistory::OpenAiChat(json!({})),
            tool_calls,
            final_text: final_text.into(),
        }
    }

    fn results(decision: LoopDecision) -> Vec<crate::protocol::tools::ExecutedToolResult> {
        let LoopDecision::Continue(results) = decision else {
            panic!("expected loop to continue");
        };
        results
    }

    #[test]
    fn weather_call_returns_sunny() {
        let mut state = ToolLoopState::new("046").expect("known loop check");
        let weather = call("get_weather", json!({"city": "Beijing"}));

        let executed = results(state.advance(&turn(vec![weather.clone()], "")));

        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].call, weather);
        assert_eq!(executed[0].output, "WEATHER_SUNNY");
        assert!(!executed[0].is_error);
    }

    #[test]
    fn time_call_returns_fixed_utc_time() {
        let mut state = ToolLoopState::new("047").expect("known loop check");
        let time = call("get_time", json!({"zone": "UTC"}));

        let executed = results(state.advance(&turn(vec![time.clone()], "")));

        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].call, time);
        assert_eq!(executed[0].output, "TIME_UTC_12:00");
        assert!(!executed[0].is_error);
    }

    #[test]
    fn check_049_returns_timeout_then_sunny() {
        let mut state = ToolLoopState::new("049").expect("known loop check");
        let first = results(state.advance(&turn(
            vec![call("get_weather", json!({"city": "Beijing"}))],
            "",
        )));
        let second = results(state.advance(&turn(
            vec![call("get_weather", json!({"city": "Beijing"}))],
            "",
        )));

        assert_eq!(first[0].output, "ERROR: timeout");
        assert!(first[0].is_error);
        assert_eq!(second[0].output, "WEATHER_SUNNY");
        assert!(!second[0].is_error);
    }

    #[test]
    fn unknown_tool_is_an_invalid_turn() {
        let mut state = ToolLoopState::new("047").expect("known loop check");

        let decision = state.advance(&turn(vec![call("delete_all", json!({}))], ""));

        let LoopDecision::Stop {
            outcome,
            contract_errors,
        } = decision
        else {
            panic!("expected invalid turn");
        };
        assert_eq!(outcome, ToolLoopOutcome::InvalidTurn);
        assert_eq!(
            contract_errors,
            ["tool_loop.unknown_tool:/tool_calls/0/name"]
        );
    }

    #[test]
    fn invalid_arguments_are_an_invalid_turn() {
        let mut state = ToolLoopState::new("046").expect("known loop check");

        let decision = state.advance(&turn(
            vec![call("get_weather", json!({"city": "Shanghai"}))],
            "",
        ));

        let LoopDecision::Stop {
            outcome,
            contract_errors,
        } = decision
        else {
            panic!("expected invalid turn");
        };
        assert_eq!(outcome, ToolLoopOutcome::InvalidTurn);
        assert_eq!(
            contract_errors,
            ["tool_loop.invalid_arguments:/tool_calls/0/arguments"]
        );
    }

    #[test]
    fn final_answer_completes_the_loop() {
        let mut state = ToolLoopState::new("048").expect("known loop check");

        let decision = state.advance(&turn(vec![], "MODEL_DOCTOR_CASE_048_OK WEATHER_SUNNY"));

        assert_eq!(decision, LoopDecision::Complete);
    }

    #[test]
    fn fourth_assistant_tool_turn_exceeds_the_bound_without_execution() {
        let mut state = ToolLoopState::new("046").expect("known loop check");
        for _ in 0..3 {
            assert!(matches!(
                state.advance(&turn(
                    vec![call("get_weather", json!({"city": "Beijing"}))],
                    ""
                )),
                LoopDecision::Continue(_)
            ));
        }

        let decision = state.advance(&turn(
            vec![call("get_weather", json!({"city": "Beijing"}))],
            "",
        ));

        assert_eq!(
            decision,
            LoopDecision::Stop {
                outcome: ToolLoopOutcome::MaxTurnsExceeded,
                contract_errors: vec![],
            }
        );
    }

    #[test]
    fn third_tool_turn_can_be_followed_by_a_fourth_final_turn() {
        let mut state = ToolLoopState::new("046").expect("known loop check");
        for _ in 0..3 {
            let executed = results(state.advance(&turn(
                vec![call("get_weather", json!({"city": "Beijing"}))],
                "",
            )));
            assert_eq!(executed[0].output, "WEATHER_SUNNY");
        }

        assert_eq!(
            state.advance(&turn(vec![], "MODEL_DOCTOR_CASE_046_OK")),
            LoopDecision::Complete
        );
    }

    #[test]
    fn fourth_tool_turn_always_hits_max_before_validation() {
        let fourth_calls = [
            call("get_weather", json!({"city": "Beijing"})),
            call("unknown", json!({})),
            call("get_weather", json!({"city": "Shanghai"})),
        ];

        for fourth_call in fourth_calls {
            let mut state = ToolLoopState::new("046").expect("known loop check");
            for _ in 0..3 {
                let _ = results(state.advance(&turn(
                    vec![call("get_weather", json!({"city": "Beijing"}))],
                    "",
                )));
            }

            assert_eq!(
                state.advance(&turn(vec![fourth_call], "")),
                LoopDecision::Stop {
                    outcome: ToolLoopOutcome::MaxTurnsExceeded,
                    contract_errors: vec![],
                }
            );
        }
    }

    #[test]
    fn terminal_decisions_are_idempotent() {
        let mut completed = ToolLoopState::new("046").expect("known loop check");
        let complete = completed.advance(&turn(vec![], "done"));
        assert_eq!(
            completed.advance(&turn(
                vec![call("get_weather", json!({"city": "Beijing"}))],
                ""
            )),
            complete
        );

        let mut invalid = ToolLoopState::new("046").expect("known loop check");
        let invalid_turn = invalid.advance(&turn(vec![call("unknown", json!({}))], ""));
        assert_eq!(invalid.advance(&turn(vec![], "done")), invalid_turn);

        let mut bounded = ToolLoopState::new("046").expect("known loop check");
        for _ in 0..3 {
            let _ = results(bounded.advance(&turn(
                vec![call("get_weather", json!({"city": "Beijing"}))],
                "",
            )));
        }
        let max = bounded.advance(&turn(
            vec![call("get_weather", json!({"city": "Beijing"}))],
            "",
        ));
        assert_eq!(bounded.advance(&turn(vec![], "done")), max);
    }

    #[test]
    fn empty_final_and_text_with_call_are_distinguished_by_calls_only() {
        let mut empty = ToolLoopState::new("046").expect("known loop check");
        assert_eq!(empty.advance(&turn(vec![], "")), LoopDecision::Complete);

        let mut with_call = ToolLoopState::new("046").expect("known loop check");
        let executed = results(with_call.advance(&turn(
            vec![call("get_weather", json!({"city": "Beijing"}))],
            "ignore this text",
        )));
        assert_eq!(executed[0].output, "WEATHER_SUNNY");
    }

    #[test]
    fn all_correlation_variants_and_non_contiguous_indices_are_preserved() {
        let calls = [
            custom_call(
                2,
                ToolCorrelation::Required("required".into()),
                "get_weather",
                json!({"city": "Beijing"}),
            ),
            custom_call(
                9,
                ToolCorrelation::Optional(Some("optional".into())),
                "get_time",
                json!({"zone": "UTC"}),
            ),
            custom_call(
                42,
                ToolCorrelation::Optional(None),
                "get_weather",
                json!({"city": "Beijing"}),
            ),
        ];
        let mut state = ToolLoopState::new("047").expect("known loop check");

        for expected in calls {
            let executed = results(state.advance(&turn(vec![expected.clone()], "")));
            assert_eq!(executed[0].call, expected);
        }
    }

    fn assert_valid_batch_executes_in_order(
        check_id: &'static str,
        calls: Vec<ToolCall>,
        expected_outputs: &[&str],
    ) {
        let mut state = ToolLoopState::new(check_id).expect("known loop check");

        let executed = results(state.advance(&turn(calls.clone(), "")));

        assert_eq!(executed.len(), calls.len());
        for ((result, expected_call), expected_output) in
            executed.iter().zip(&calls).zip(expected_outputs)
        {
            assert_eq!(&result.call, expected_call);
            assert_eq!(result.output, *expected_output);
            assert!(!result.is_error);
        }
    }

    #[test]
    fn check_046_executes_multiple_valid_weather_calls_in_input_order() {
        assert_valid_batch_executes_in_order(
            "046",
            vec![
                custom_call(
                    7,
                    ToolCorrelation::Optional(None),
                    "get_weather",
                    json!({"city": "Beijing"}),
                ),
                custom_call(
                    2,
                    ToolCorrelation::Required("weather-second".into()),
                    "get_weather",
                    json!({"city": "Beijing"}),
                ),
            ],
            &["WEATHER_SUNNY", "WEATHER_SUNNY"],
        );
    }

    #[test]
    fn check_047_executes_multiple_valid_allowlisted_calls_in_input_order() {
        assert_valid_batch_executes_in_order(
            "047",
            vec![
                custom_call(
                    10,
                    ToolCorrelation::Optional(Some("weather-first".into())),
                    "get_weather",
                    json!({"city": "Beijing"}),
                ),
                custom_call(
                    3,
                    ToolCorrelation::Required("time-second".into()),
                    "get_time",
                    json!({"zone": "UTC"}),
                ),
                custom_call(
                    8,
                    ToolCorrelation::Optional(None),
                    "get_weather",
                    json!({"city": "Beijing"}),
                ),
            ],
            &["WEATHER_SUNNY", "TIME_UTC_12:00", "WEATHER_SUNNY"],
        );
    }

    #[test]
    fn check_048_executes_multiple_valid_weather_calls_in_input_order() {
        assert_valid_batch_executes_in_order(
            "048",
            vec![
                custom_call(
                    5,
                    ToolCorrelation::Required("weather-first".into()),
                    "get_weather",
                    json!({"city": "Beijing"}),
                ),
                custom_call(
                    1,
                    ToolCorrelation::Optional(Some("weather-second".into())),
                    "get_weather",
                    json!({"city": "Beijing"}),
                ),
            ],
            &["WEATHER_SUNNY", "WEATHER_SUNNY"],
        );
    }

    #[test]
    fn valid_multi_call_check_rejects_an_entire_batch_when_one_call_is_invalid() {
        let mut state = ToolLoopState::new("046").expect("known loop check");

        let decision = state.advance(&turn(
            vec![
                custom_call(
                    4,
                    ToolCorrelation::Required("valid-first".into()),
                    "get_weather",
                    json!({"city": "Beijing"}),
                ),
                custom_call(
                    9,
                    ToolCorrelation::Optional(None),
                    "get_weather",
                    json!({"city": "Shanghai"}),
                ),
            ],
            "",
        ));

        assert_eq!(
            decision,
            LoopDecision::Stop {
                outcome: ToolLoopOutcome::InvalidTurn,
                contract_errors: vec!["tool_loop.invalid_arguments:/tool_calls/1/arguments".into()],
            }
        );
        assert_eq!(state.weather_attempts, 0);
    }

    #[test]
    fn invalid_batch_is_atomic_and_does_not_consume_049_attempt() {
        let mut state = ToolLoopState::new("049").expect("known loop check");
        let decision = state.advance(&turn(
            vec![
                call("get_weather", json!({"city": "Beijing"})),
                call("get_weather", json!({"city": "Shanghai"})),
            ],
            "",
        ));

        let LoopDecision::Stop {
            outcome,
            contract_errors,
        } = decision
        else {
            panic!("expected invalid turn");
        };
        assert_eq!(outcome, ToolLoopOutcome::InvalidTurn);
        assert!(
            contract_errors
                .iter()
                .any(|error| error == "tool_loop.multiple_tool_calls:/tool_calls")
        );
        assert!(
            contract_errors
                .iter()
                .any(|error| { error == "tool_loop.invalid_arguments:/tool_calls/1/arguments" })
        );
        assert_eq!(state.weather_attempts, 0);
    }

    #[test]
    fn check_specific_tool_allowlists_are_enforced() {
        assert!(ToolLoopState::new("045").is_none());
        assert!(ToolLoopState::new("050").is_none());

        for check_id in ["046", "047", "048", "049"] {
            let mut state = ToolLoopState::new(check_id).expect("known loop check");
            assert!(matches!(
                state.advance(&turn(
                    vec![call("get_weather", json!({"city": "Beijing"}))],
                    ""
                )),
                LoopDecision::Continue(_)
            ));
        }

        let mut allowed_time = ToolLoopState::new("047").expect("known loop check");
        assert!(matches!(
            allowed_time.advance(&turn(vec![call("get_time", json!({"zone": "UTC"}))], "")),
            LoopDecision::Continue(_)
        ));

        for check_id in ["046", "048", "049"] {
            let mut state = ToolLoopState::new(check_id).expect("known loop check");
            let LoopDecision::Stop {
                outcome,
                contract_errors,
            } = state.advance(&turn(vec![call("get_time", json!({"zone": "UTC"}))], ""))
            else {
                panic!("expected invalid turn");
            };
            assert_eq!(outcome, ToolLoopOutcome::InvalidTurn);
            assert_eq!(
                contract_errors,
                ["tool_loop.tool_not_allowed:/tool_calls/0/name"]
            );
        }
    }

    #[test]
    fn check_049_same_turn_retry_is_rejected_without_consuming_attempt() {
        let mut state = ToolLoopState::new("049").expect("known loop check");

        let decision = state.advance(&turn(
            vec![
                call("get_weather", json!({"city": "Beijing"})),
                call("get_weather", json!({"city": "Beijing"})),
            ],
            "",
        ));

        let LoopDecision::Stop {
            outcome,
            contract_errors,
        } = decision
        else {
            panic!("expected invalid turn");
        };
        assert_eq!(outcome, ToolLoopOutcome::InvalidTurn);
        assert_eq!(
            contract_errors,
            ["tool_loop.multiple_tool_calls:/tool_calls"]
        );
        assert_eq!(state.weather_attempts, 0);
    }

    #[test]
    fn exact_argument_objects_are_required() {
        let invalid_arguments = [
            json!(null),
            json!([]),
            json!({}),
            json!({"city": "Beijing", "unit": "C"}),
            json!({"zone": "UTC", "extra": true}),
        ];

        for arguments in invalid_arguments {
            let (check_id, name) = if arguments.get("zone").is_some() {
                ("047", "get_time")
            } else {
                ("046", "get_weather")
            };
            let mut state = ToolLoopState::new(check_id).expect("known loop check");
            let LoopDecision::Stop {
                outcome,
                contract_errors,
            } = state.advance(&turn(vec![call(name, arguments)], ""))
            else {
                panic!("expected invalid turn");
            };
            assert_eq!(outcome, ToolLoopOutcome::InvalidTurn);
            assert_eq!(contract_errors.len(), 1);
            assert!(!contract_errors[0].is_empty());
        }
    }
}
