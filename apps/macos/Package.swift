// swift-tools-version:5.9
// SPDX-License-Identifier: GPL-3.0-or-later
import PackageDescription

let package = Package(
    name: "SolarCraftMac",
    platforms: [.macOS(.v14)], // spike; product deployment target is macOS 26 (ADR 0001)
    targets: [
        .systemLibrary(name: "CSolarCraft", path: "Sources/CSolarCraft"),
        .executableTarget(name: "SolarCraftMac", dependencies: ["CSolarCraft"], path: "Sources/SolarCraftMac"),
    ]
)
