// swift-tools-version: 5.7

import PackageDescription

let package = Package(
    name: "ScreenCaptureKitAudioSpike",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(name: "SpikeAudioCapture", targets: ["SpikeAudioCapture"])
    ],
    targets: [
        .executableTarget(
            name: "SpikeAudioCapture",
            linkerSettings: [
                .linkedFramework("AVFoundation"),
                .linkedFramework("CoreGraphics"),
                .linkedFramework("CoreMedia"),
                .linkedFramework("ScreenCaptureKit")
            ]
        )
    ]
)
