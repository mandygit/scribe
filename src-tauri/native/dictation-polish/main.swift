import Foundation
import FoundationModels

// Dictation polish via Apple Intelligence (FoundationModels). Reads raw dictated
// text on stdin, prints cleaned, inject-ready text on stdout.
//
// We avoid the @Generable guided-generation macro because it requires full
// Xcode (the FoundationModelsMacros plugin). Instead we use plain generation
// plus stripPreamble(), which removes any leading preamble line and surrounding
// quotes so only the cleaned text remains.
//
// Exit codes: 0 ok, 2 Apple Intelligence unavailable, 1 generation error.

func stripPreamble(_ input: String) -> String {
    var t = input.trimmingCharacters(in: .whitespacesAndNewlines)
    let markers = ["here is", "here's", "sure", "cleaned", "certainly", "okay", "of course"]

    if let nl = t.firstIndex(of: "\n") {
        let firstLine = t[..<nl].lowercased().trimmingCharacters(in: .whitespaces)
        if firstLine.hasSuffix(":") || markers.contains(where: { firstLine.hasPrefix($0) }) {
            t = String(t[t.index(after: nl)...]).trimmingCharacters(in: .whitespacesAndNewlines)
        }
    } else if let colon = t.firstIndex(of: ":") {
        let head = t[..<colon].lowercased()
        if markers.contains(where: { head.hasPrefix($0) }) {
            t = String(t[t.index(after: colon)...]).trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    if t.count >= 2 {
        let first = t.first!, last = t.last!
        if (first == "\"" && last == "\"") || (first == "'" && last == "'")
            || (first == "\u{201C}" && last == "\u{201D}") {
            t = String(t.dropFirst().dropLast()).trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }
    return t
}

@main
struct DictationPolish {
    static func main() async {
        let raw = String(data: FileHandle.standardInput.readDataToEndOfFile(), encoding: .utf8) ?? ""
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            return
        }

        if case .unavailable(let reason) = SystemLanguageModel.default.availability {
            FileHandle.standardError.write(Data("apple_intelligence_unavailable: \(reason)\n".utf8))
            exit(2)
        }

        let instructions = """
        You clean up dictated text. Fix punctuation, capitalize sentences, proper nouns, and acronyms, \
        remove filler words (um, uh, like, you know), and turn spoken "first/second/third" enumerations \
        into a numbered list. Keep the speaker's meaning and words. Output ONLY the cleaned text — no \
        preamble, labels, quotes, or explanation.
        """
        let session = LanguageModelSession(instructions: instructions)
        do {
            let response = try await session.respond(to: trimmed, options: GenerationOptions(temperature: 0.1))
            print(stripPreamble(response.content))
        } catch {
            FileHandle.standardError.write(Data("generation_error: \(error)\n".utf8))
            exit(1)
        }
    }
}
