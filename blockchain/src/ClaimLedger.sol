// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ClaimLedger
/// @notice Append-only, on-chain registry of claim-event fingerprints.
/// @dev Stores NO personal or health data: only 32-byte hashes of off-chain records
///      (which are encrypted and optionally pinned to IPFS) plus the IPFS CID. Anyone
///      can verify that a record existed, unchanged, at a given block; nobody can
///      read the claim from the chain. Entries can never be edited or deleted.
contract ClaimLedger {
    struct Entry {
        bytes32 dataHash;
        uint8 kind;
        uint64 timestamp;
        address submitter;
        string cid;
    }

    uint8 public constant MAX_KIND = 8;
    uint256 public constant MAX_CID_LENGTH = 128;
    uint256 public constant MAX_BATCH = 100;

    address public owner;
    address public pendingOwner;
    bool public paused;
    uint256 public totalEntries;

    mapping(address => bool) public writers;
    mapping(bytes32 => bool) public isAnchored;
    mapping(bytes32 => Entry[]) private _history;

    event EntryAnchored(
        bytes32 indexed recordId, uint256 indexed index, bytes32 indexed dataHash, uint8 kind, string cid, address submitter
    );
    event WriterSet(address indexed account, bool allowed);
    event OwnershipTransferStarted(address indexed previousOwner, address indexed newOwner);
    event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    event PausedSet(bool paused);

    error NotOwner();
    error NotWriter();
    error NotPendingOwner();
    error ContractPaused();
    error ZeroAddress();
    error ZeroHash();
    error AlreadyAnchored(bytes32 dataHash);
    error InvalidKind(uint8 kind);
    error CidTooLong();
    error IndexOutOfRange();
    error LengthMismatch();
    error BatchTooLarge();

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier onlyWriter() {
        if (!writers[msg.sender]) revert NotWriter();
        _;
    }

    modifier whenNotPaused() {
        if (paused) revert ContractPaused();
        _;
    }

    /// @param initialWriter backend signer allowed to anchor (in addition to the deployer)
    constructor(address initialWriter) {
        owner = msg.sender;
        emit OwnershipTransferred(address(0), msg.sender);
        writers[msg.sender] = true;
        emit WriterSet(msg.sender, true);
        if (initialWriter != address(0) && initialWriter != msg.sender) {
            writers[initialWriter] = true;
            emit WriterSet(initialWriter, true);
        }
    }

    // ------------------------------------------------------------------ writes

    function anchor(bytes32 recordId, bytes32 dataHash, uint8 kind, string calldata cid)
        external
        onlyWriter
        whenNotPaused
        returns (uint256 index)
    {
        return _anchor(recordId, dataHash, kind, cid);
    }

    /// @notice Anchor several entries in one transaction (no CIDs) to save gas.
    function anchorBatch(bytes32[] calldata recordIds, bytes32[] calldata dataHashes, uint8[] calldata kinds)
        external
        onlyWriter
        whenNotPaused
    {
        uint256 n = recordIds.length;
        if (n != dataHashes.length || n != kinds.length) revert LengthMismatch();
        if (n > MAX_BATCH) revert BatchTooLarge();
        for (uint256 i = 0; i < n; ++i) {
            _anchor(recordIds[i], dataHashes[i], kinds[i], "");
        }
    }

    function _anchor(bytes32 recordId, bytes32 dataHash, uint8 kind, string memory cid) internal returns (uint256 index) {
        if (dataHash == bytes32(0) || recordId == bytes32(0)) revert ZeroHash();
        if (isAnchored[dataHash]) revert AlreadyAnchored(dataHash);
        if (kind == 0 || kind > MAX_KIND) revert InvalidKind(kind);
        if (bytes(cid).length > MAX_CID_LENGTH) revert CidTooLong();

        isAnchored[dataHash] = true;
        Entry[] storage history = _history[recordId];
        index = history.length;
        history.push(Entry({dataHash: dataHash, kind: kind, timestamp: uint64(block.timestamp), submitter: msg.sender, cid: cid}));
        unchecked {
            ++totalEntries;
        }
        emit EntryAnchored(recordId, index, dataHash, kind, cid, msg.sender);
    }

    // ------------------------------------------------------------------ reads

    function entryCount(bytes32 recordId) external view returns (uint256) {
        return _history[recordId].length;
    }

    function getEntry(bytes32 recordId, uint256 index) external view returns (Entry memory) {
        if (index >= _history[recordId].length) revert IndexOutOfRange();
        return _history[recordId][index];
    }

    function latest(bytes32 recordId) external view returns (Entry memory) {
        uint256 n = _history[recordId].length;
        if (n == 0) revert IndexOutOfRange();
        return _history[recordId][n - 1];
    }

    /// @notice True when `dataHash` is the entry at `index` of `recordId`.
    function verify(bytes32 recordId, uint256 index, bytes32 dataHash) external view returns (bool) {
        if (index >= _history[recordId].length) return false;
        return _history[recordId][index].dataHash == dataHash;
    }

    // ------------------------------------------------------------------ admin

    function setWriter(address account, bool allowed) external onlyOwner {
        if (account == address(0)) revert ZeroAddress();
        writers[account] = allowed;
        emit WriterSet(account, allowed);
    }

    function setPaused(bool value) external onlyOwner {
        paused = value;
        emit PausedSet(value);
    }

    /// @notice Two-step ownership transfer: the new owner must accept.
    function transferOwnership(address newOwner) external onlyOwner {
        if (newOwner == address(0)) revert ZeroAddress();
        pendingOwner = newOwner;
        emit OwnershipTransferStarted(owner, newOwner);
    }

    function acceptOwnership() external {
        if (msg.sender != pendingOwner) revert NotPendingOwner();
        emit OwnershipTransferred(owner, msg.sender);
        owner = msg.sender;
        pendingOwner = address(0);
    }
}
