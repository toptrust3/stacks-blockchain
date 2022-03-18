;; The .subnets contract
;; Error codes
(define-constant ERR_BLOCK_ALREADY_COMMITTED 1)
(define-constant ERR_INVALID_MINER 2)
(define-constant ERR_VALIDATION_FAILED 3)

;; Map from Stacks block height to block commit
(define-map block-commits uint (buff 32))
(define-constant miners (list 'SPAXYA5XS51713FDTQ8H94EJ4V579CXMTRNBZKSF 'SP3X6QWWETNBZWGBK6DRGTR1KX50S74D3433WDGJY 'ST1AW6EKPGT61SQ9FNVDS17RKNWT8ZP582VF9HSCP))

;; maps a principal to an optional principal
(define-private (map-to-optional (address principal))
    (some address)
)

;; if a == b, return none; else return a
(define-private (is-principal-eq (miner-a (optional principal)) (miner-b (optional principal)))
    (if (is-eq miner-a miner-b)
        none
        miner-b
    )
)

;; Returns a boolean indicating whether the given principal is in the list of miners
(define-private (is-miner (miner principal))
   (let (
        (mapped-miners (map map-to-optional miners))
        (mapped-miner (map-to-optional miner))
        (is-in-list (fold is-principal-eq mapped-miners mapped-miner))
   )
        (is-none is-in-list)
   )
)

;; Determines whether the commit-block operated can be carried out
(define-private (can-commit-block? (block (buff 32)) (commit-block-height uint))
    (begin
        ;; check no block has been committed at this height
        (asserts! (is-none (map-get? block-commits commit-block-height)) (err ERR_BLOCK_ALREADY_COMMITTED))

        ;; check that the tx sender is one of the miners
        (asserts! (is-miner tx-sender) (err ERR_INVALID_MINER))

        (ok true)
    )
)

;; Modifies the block-commits map with a new commit and has a print
(define-private (inner-commit-block (block (buff 32)) (commit-block-height uint))
    (begin
        (map-set block-commits commit-block-height block)
        (print { event: "block-commit", block-commit: block})
        (ok block)
    )
)

;; Subnets miners call this to commit a block at a particular height
(define-public (commit-block (block (buff 32)) (commit-block-height uint))
    (begin
        (unwrap! (can-commit-block? block commit-block-height) (err ERR_VALIDATION_FAILED))
        (inner-commit-block block commit-block-height)
    )
)


;; Implement functions below in M2
;; user: deposit asset

;; miner: acknowledge deposit

;; user: issue withdraw request

;; miner: approve withdraw