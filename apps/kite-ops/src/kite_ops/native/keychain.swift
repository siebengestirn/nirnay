import Foundation
import Security
import Darwin

func fail(_ message: String, _ code: Int32 = 2) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

let args = CommandLine.arguments
if args.count != 4 {
    fail("usage: keychain.swift get|set|delete SERVICE ACCOUNT")
}
let operation = args[1]
let service = args[2]
let account = args[3]
if service.isEmpty || account.isEmpty { fail("service and account are required") }

let base: [String: Any] = [
    kSecClass as String: kSecClassGenericPassword,
    kSecAttrService as String: service,
    kSecAttrAccount as String: account,
]

switch operation {
case "get":
    var query = base
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    guard status == errSecSuccess, let data = result as? Data else {
        fail("Keychain item not found or inaccessible (OSStatus \(status))", 3)
    }
    FileHandle.standardOutput.write(data)
case "set":
    let secret = FileHandle.standardInput.readDataToEndOfFile()
    guard !secret.isEmpty else { fail("refusing to store an empty secret") }
    var attributes = base
    attributes[kSecValueData as String] = secret
    attributes[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
    let addStatus = SecItemAdd(attributes as CFDictionary, nil)
    if addStatus == errSecDuplicateItem {
        let updateStatus = SecItemUpdate(base as CFDictionary,
                                         [kSecValueData as String: secret] as CFDictionary)
        guard updateStatus == errSecSuccess else {
            fail("Keychain update failed (OSStatus \(updateStatus))", 4)
        }
    } else if addStatus != errSecSuccess {
        fail("Keychain add failed (OSStatus \(addStatus))", 4)
    }
case "delete":
    let status = SecItemDelete(base as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else {
        fail("Keychain delete failed (OSStatus \(status))", 4)
    }
default:
    fail("unsupported operation")
}
