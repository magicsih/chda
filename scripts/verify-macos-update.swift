// Verify Sparkle's Ed25519 archive signature using the signed app's public key.
import CryptoKit
import Foundation

guard CommandLine.arguments.count == 4,
      let signature = Data(base64Encoded: CommandLine.arguments[2]), signature.count == 64,
      let publicKey = Data(base64Encoded: CommandLine.arguments[3]), publicKey.count == 32 else {
    fputs("Invalid Ed25519 verification inputs\n", stderr)
    exit(1)
}
do {
    let url = URL(fileURLWithPath: CommandLine.arguments[1])
    let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
    guard size > 0, size <= 2 * 1024 * 1024 * 1024 else {
        fputs("Release archive size exceeds the verification limit\n", stderr)
        exit(1)
    }
    let key = try Curve25519.Signing.PublicKey(rawRepresentation: publicKey)
    let archive = try Data(contentsOf: url, options: .mappedIfSafe)
    guard key.isValidSignature(signature, for: archive) else {
        fputs("Release archive Ed25519 signature mismatch\n", stderr)
        exit(1)
    }
} catch {
    fputs("Unable to verify release archive signature\n", stderr)
    exit(1)
}
