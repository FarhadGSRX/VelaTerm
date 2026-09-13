// swift-tools-version: 5.9
import PackageDescription
let package = Package(name: "VelaRemotePlugin", platforms: [.iOS(.v17)], products: [.library(name: "VelatermCapacitorRemote", targets: ["VelaRemotePlugin"])], dependencies: [
    .package(url: "https://github.com/ionic-team/capacitor-swift-pm.git", exact: "8.5.1"),
    .package(url: "https://github.com/orlandos-nl/Citadel.git", revision: "ae8562f895de06ccb86fdb1cbb65fd99c8976e12")
], targets: [.target(name: "VelaRemotePlugin", dependencies: [.product(name: "Capacitor", package: "capacitor-swift-pm"), .product(name: "Citadel", package: "Citadel")], path: "ios/Sources/VelaRemotePlugin", resources: [.copy("Bootstrap")])])
