import Foundation

struct LaunchConfiguration {
  var endpoint: String
  var model: String
  var modules: [String]?
  var stopAfter: String?
  var timeoutSeconds: Int
  var outputPath: String?
  var reportDirectory: String?
  var htmlPath: String?
  var cliPath: String

  static func from(arguments: [String]) -> LaunchConfiguration? {
    func value(_ name: String) -> String? {
      guard let index = arguments.firstIndex(of: name), index + 1 < arguments.count else {
        return nil
      }
      return arguments[index + 1]
    }
    guard let endpoint = value("--agentcheck-endpoint"),
      let model = value("--agentcheck-model"),
      let cliPath = value("--agentcheck-cli-path")
    else { return nil }
    return LaunchConfiguration(
      endpoint: endpoint,
      model: model,
      modules: value("--agentcheck-modules")?.split(separator: ",").map(String.init),
      stopAfter: value("--agentcheck-stop-after"),
      timeoutSeconds: Int(value("--agentcheck-timeout-seconds") ?? "300") ?? 300,
      outputPath: value("--agentcheck-output"),
      reportDirectory: value("--agentcheck-report-dir"),
      htmlPath: value("--agentcheck-html"),
      cliPath: cliPath
    )
  }
}

struct ProgressEvent: Decodable {
  var phase: String
  var moduleID: String?
  var index: Int
  var total: Int
  var itemIndex: Int?
  var itemTotal: Int?
  var itemName: String?
  var state: String?
  var message: String

  enum CodingKeys: String, CodingKey {
    case phase
    case moduleID = "module_id"
    case index
    case total
    case itemIndex = "item_index"
    case itemTotal = "item_total"
    case itemName = "item_name"
    case state
    case message
  }

  init(from decoder: Decoder) throws {
    let values = try decoder.container(keyedBy: CodingKeys.self)
    phase = try values.decode(String.self, forKey: .phase)
    moduleID = try values.decodeIfPresent(String.self, forKey: .moduleID)
    index = try values.decode(Int.self, forKey: .index)
    total = try values.decode(Int.self, forKey: .total)
    itemIndex = try values.decodeIfPresent(Int.self, forKey: .itemIndex)
    itemTotal = try values.decodeIfPresent(Int.self, forKey: .itemTotal)
    itemName = try values.decodeIfPresent(String.self, forKey: .itemName)
    state = try values.decodeIfPresent(String.self, forKey: .state)
    message = try values.decode(String.self, forKey: .message)
  }
}

extension CheckModule {
  static func fromBackend(_ backendID: String) -> CheckModule? {
    switch backendID {
    case "specification": return .parameters
    case "capability": return .functions
    case "performance": return .performance
    case "agent": return .agent
    case "baseline": return .comparison
    default: return nil
    }
  }

  var backendID: String? {
    switch self {
    case .info: return nil
    case .parameters: return "specification"
    case .functions: return "capability"
    case .performance: return "performance"
    case .agent: return "agent"
    case .comparison: return "baseline"
    }
  }
}
