import AVFoundation
import CoreGraphics
import CoreMedia
import Foundation
import ScreenCaptureKit

enum SystemAudioCaptureError: LocalizedError {
    case missingOutputPath
    case screenRecordingDenied
    case noDisplayAvailable
    case cannotRemoveExistingOutput(URL, Error)
    case cannotCreateWriter(URL, Error)
    case cannotAddAudioInput
    case cannotStartWriter(Error?)
    case shareableContentFailed(Error)
    case captureStopped(Error)
    case noAudioBuffersCaptured

    var errorDescription: String? {
        switch self {
        case .missingOutputPath:
            return "Missing output path argument for system audio capture."
        case .screenRecordingDenied:
            return "Screen Recording permission is required to capture system audio. Enable Resonance in System Settings -> Privacy & Security -> Screen Recording, then relaunch."
        case .noDisplayAvailable:
            return "ScreenCaptureKit did not report any displays to capture."
        case let .cannotRemoveExistingOutput(url, error):
            return "Could not remove existing system audio output at \(url.path): \(error.localizedDescription)"
        case let .cannotCreateWriter(url, error):
            return "Could not create system audio writer at \(url.path): \(error.localizedDescription)"
        case .cannotAddAudioInput:
            return "Could not add the system audio input to AVAssetWriter."
        case let .cannotStartWriter(error):
            return "Could not start AVAssetWriter: \(error?.localizedDescription ?? "unknown writer error")"
        case let .shareableContentFailed(error):
            return "ScreenCaptureKit could not enumerate shareable displays/windows: \(error.localizedDescription)"
        case let .captureStopped(error):
            return "ScreenCaptureKit stopped system audio capture: \(error.localizedDescription)"
        case .noAudioBuffersCaptured:
            return "System audio capture stopped without receiving audio buffers. Play meeting audio and try again."
        }
    }
}

final class SystemAudioRecorder: NSObject, SCStreamOutput, SCStreamDelegate {
    private let outputURL: URL
    private let writer: AVAssetWriter
    private let audioInput: AVAssetWriterInput
    private let queue = DispatchQueue(label: "com.resonance.meetingcoach.system-audio")
    private let stateLock = NSLock()
    private var hasStartedWriting = false
    private var capturedAudioBuffers = 0
    private var stoppedError: Error?

    init(outputURL: URL) throws {
        self.outputURL = outputURL

        if FileManager.default.fileExists(atPath: outputURL.path) {
            do {
                try FileManager.default.removeItem(at: outputURL)
            } catch {
                throw SystemAudioCaptureError.cannotRemoveExistingOutput(outputURL, error)
            }
        }

        do {
            writer = try AVAssetWriter(outputURL: outputURL, fileType: .m4a)
        } catch {
            throw SystemAudioCaptureError.cannotCreateWriter(outputURL, error)
        }

        audioInput = AVAssetWriterInput(mediaType: .audio, outputSettings: [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 48_000.0,
            AVNumberOfChannelsKey: 2,
            AVEncoderBitRateKey: 128_000
        ])
        audioInput.expectsMediaDataInRealTime = true

        guard writer.canAdd(audioInput) else {
            throw SystemAudioCaptureError.cannotAddAudioInput
        }
        writer.add(audioInput)

        super.init()
    }

    var outputQueue: DispatchQueue {
        queue
    }

    var receivedAudioBuffers: Int {
        stateLock.lock()
        defer { stateLock.unlock() }
        return capturedAudioBuffers
    }

    var streamError: Error? {
        stateLock.lock()
        defer { stateLock.unlock() }
        return stoppedError
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of outputType: SCStreamOutputType) {
        guard outputType == .audio, sampleBuffer.isValid, CMSampleBufferDataIsReady(sampleBuffer) else {
            return
        }

        if !hasStartedWriting {
            let startTime = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
            guard writer.startWriting() else {
                stateLock.lock()
                stoppedError = SystemAudioCaptureError.cannotStartWriter(writer.error)
                stateLock.unlock()
                return
            }
            writer.startSession(atSourceTime: startTime)
            hasStartedWriting = true
        }

        guard audioInput.isReadyForMoreMediaData else {
            return
        }

        if audioInput.append(sampleBuffer) {
            stateLock.lock()
            capturedAudioBuffers += 1
            stateLock.unlock()
        } else if let error = writer.error {
            stateLock.lock()
            stoppedError = error
            stateLock.unlock()
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        stateLock.lock()
        stoppedError = error
        stateLock.unlock()
    }

    func finish() async throws {
        audioInput.markAsFinished()

        await withCheckedContinuation { continuation in
            writer.finishWriting {
                continuation.resume()
            }
        }

        if let error = writer.error {
            throw error
        }
    }
}

@main
struct SystemAudioCapture {
    static func main() async {
        do {
            try await run()
        } catch {
            fputs("ERROR: \(error.localizedDescription)\n", stderr)
            exit(EXIT_FAILURE)
        }
    }

    private static func run() async throws {
        guard CommandLine.arguments.count == 2 else {
            throw SystemAudioCaptureError.missingOutputPath
        }

        try ensureScreenRecordingPermission()

        let outputURL = URL(fileURLWithPath: CommandLine.arguments[1])
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        } catch {
            throw SystemAudioCaptureError.shareableContentFailed(error)
        }

        guard let display = content.displays.first else {
            throw SystemAudioCaptureError.noDisplayAvailable
        }

        let recorder = try SystemAudioRecorder(outputURL: outputURL)
        let configuration = SCStreamConfiguration()
        configuration.width = 2
        configuration.height = 2
        configuration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
        configuration.queueDepth = 3
        configuration.showsCursor = false
        configuration.capturesAudio = true
        configuration.excludesCurrentProcessAudio = true
        configuration.sampleRate = 48_000
        configuration.channelCount = 2

        let filter = SCContentFilter(display: display, excludingWindows: [])
        let stream = SCStream(filter: filter, configuration: configuration, delegate: recorder)

        try stream.addStreamOutput(recorder, type: .screen, sampleHandlerQueue: recorder.outputQueue)
        try stream.addStreamOutput(recorder, type: .audio, sampleHandlerQueue: recorder.outputQueue)
        try await stream.startCapture()

        _ = readLine()

        try await stream.stopCapture()
        if let streamError = recorder.streamError {
            throw SystemAudioCaptureError.captureStopped(streamError)
        }
        guard recorder.receivedAudioBuffers > 0 else {
            throw SystemAudioCaptureError.noAudioBuffersCaptured
        }
        try await recorder.finish()
    }

    private static func ensureScreenRecordingPermission() throws {
        if CGPreflightScreenCaptureAccess() {
            return
        }

        let granted = CGRequestScreenCaptureAccess()
        guard granted else {
            throw SystemAudioCaptureError.screenRecordingDenied
        }
    }
}
