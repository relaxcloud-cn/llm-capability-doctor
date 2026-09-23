import SwiftUI

struct ComparisonView: View {
  @ObservedObject var store: Workbench
  var body: some View { ModuleView(store: store, module: .comparison) }
}
