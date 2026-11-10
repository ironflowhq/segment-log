122 traced processes and 24 owner processes, 0 failed.

## Exact resident peaks

### 16k

16,384 records, 0 keys, 1 streams, 256 records in each group, 32 payload bytes. KiB, median [range] of 3 processes in each cell; pinned and unpinned processes together.

| Mode | Barrier | Phase that ends there | Steady RSS | Exact phase peak | Peak above both ends | Peak above baseline | Mark low, unpinned | Mark low, pinned |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `touched_held` | allocate and fill one buffer | 2,328 [2,328–2,332] | 2,328 [2,328–2,332] | 0 | 516 | 0 | 0 |
| touch | `touched_dropped` | check and drop the buffer | 1,812 [1,812–1,816] | 2,328 [2,328–2,332] | 0 | 516 | 120 [120–124] | 120 [120–124] |
| caller | `caller_held` | build every submit | 4,506 [4,488–4,508] | 4,506 [4,488–4,508] | 0 | 2,692 | 0 | 0 |
| caller | `caller_dropped` | drop every submit | 4,506 [4,488–4,508] | 4,506 [4,488–4,508] | 0 | 2,692 | 0 | 0 |
| empty | `empty_held` | open a new log | 2,192 [2,192–2,196] | 2,192 [2,192–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_held` | read the empty log | 2,192 [2,192–2,196] | 2,192 [2,192–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_dropped` | drop the empty result | 2,192 [2,192–2,196] | 2,192 [2,192–2,196] | 0 | 380 | 0 | 0 |
| empty | `engine_dropped` | drop the engine | 2,092 [2,092–2,096] | 2,192 [2,192–2,196] | 0 | 380 | 100 | 100 |
| append | `empty_held` | open a new log | 2,194 [2,192–2,196] | 2,194 [2,192–2,196] | 0 | 380 | 0 | 0 |
| append | `appended_held` | append every group | 2,278 [2,276–2,280] | 2,278 [2,276–2,280] | 0 | 464 | 0 | 0 |
| append | `read_one_held` | read one record | 2,292 [2,288–2,296] | 2,292 [2,288–2,296] | 0 | 478 [476–480] | 0 | 0 |
| append | `read_one_dropped` | check and drop one record | 2,292 [2,288–2,296] | 2,292 [2,288–2,296] | 0 | 478 [476–480] | 0 | 0 |
| append | `validation_held` | read every record | 4,958 [4,956–4,960] | 4,958 [4,956–4,960] | 0 | 3,144 | 0 | 0 |
| append | `validation_dropped` | check and drop every record | 3,546 [3,544–3,548] | 4,958 [4,956–4,960] | 0 | 3,144 | 192 [188–192] | 188 [64–188] |
| append | `engine_dropped` | drop the engine | 3,446 [3,444–3,448] | 3,546 [3,544–3,548] | 0 | 1,732 | 100 | 64 |
| recover | `recovered_held` | reopen the log | 2,236 [2,220–2,240] | 2,236 [2,220–2,240] | 0 | 424 [424–428] | 0 | 0 |
| recover | `read_one_held` | read one record | 2,236 [2,220–2,240] | 2,236 [2,220–2,240] | 0 | 424 [424–428] | 0 | 0 |
| recover | `read_one_dropped` | check and drop one record | 2,236 [2,220–2,240] | 2,236 [2,220–2,240] | 0 | 424 [424–428] | 0 | 0 |
| recover | `validation_held` | read every record | 4,916 [4,900–4,920] | 4,916 [4,900–4,920] | 0 | 3,104 [3,104–3,108] | 0 | 0 |
| recover | `validation_dropped` | check and drop every record | 3,504 [3,488–3,508] | 4,916 [4,900–4,920] | 0 | 3,104 [3,104–3,108] | 148 [148–152] | 148 [148–152] |
| recover | `engine_dropped` | drop the engine | 3,404 [3,388–3,408] | 3,504 [3,488–3,508] | 0 | 1,692 [1,692–1,696] | 76 | 64 |

### 16k-keyed

16,384 records, 16,384 keys, 1 streams, 256 records in each group, 32 payload bytes. KiB, median [range] of 3 processes in each cell; pinned and unpinned processes together.

| Mode | Barrier | Phase that ends there | Steady RSS | Exact phase peak | Peak above both ends | Peak above baseline | Mark low, unpinned | Mark low, pinned |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `touched_held` | allocate and fill one buffer | 2,328 [2,320–2,328] | 2,328 [2,320–2,328] | 0 | 516 | 0 | 0 |
| touch | `touched_dropped` | check and drop the buffer | 1,812 [1,804–1,812] | 2,328 [2,320–2,328] | 0 | 516 | 120 | 120 |
| caller | `caller_held` | build every submit | 5,016 [5,000–5,020] | 5,016 [5,000–5,020] | 0 | 3,204 | 0 | 0 |
| caller | `caller_dropped` | drop every submit | 5,016 [5,000–5,020] | 5,016 [5,000–5,020] | 0 | 3,204 | 0 | 0 |
| empty | `empty_held` | open a new log | 2,190 [2,180–2,196] | 2,190 [2,180–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_held` | read the empty log | 2,190 [2,180–2,196] | 2,190 [2,180–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_dropped` | drop the empty result | 2,190 [2,180–2,196] | 2,190 [2,180–2,196] | 0 | 380 | 0 | 0 |
| empty | `engine_dropped` | drop the engine | 2,090 [2,080–2,096] | 2,190 [2,180–2,196] | 0 | 380 | 100 | 100 |
| append | `empty_held` | open a new log | 2,192 [2,184–2,196] | 2,192 [2,184–2,196] | 0 | 380 | 0 | 0 |
| append | `appended_held` | append every group | 4,008 [4,004–4,016] | 4,008 [4,004–4,016] | 0 | 2,198 [2,196–2,200] | 0 | 0 |
| append | `read_one_held` | read one record | 4,010 [4,008–4,024] | 4,010 [4,008–4,024] | 0 | 2,202 [2,196–2,208] | 0 | 0 |
| append | `read_one_dropped` | check and drop one record | 4,010 [4,008–4,024] | 4,010 [4,008–4,024] | 0 | 2,202 [2,196–2,208] | 0 | 0 |
| append | `validation_held` | read every record | 7,192 [7,184–7,196] | 7,192 [7,184–7,196] | 0 | 5,380 | 0 | 0 |
| append | `validation_dropped` | check and drop every record | 5,780 [5,772–5,784] | 7,192 [7,184–7,196] | 0 | 5,380 | 248 [248–376] | 120 [120–124] |
| append | `engine_dropped` | drop the engine | 5,680 [5,672–5,684] | 5,780 [5,772–5,784] | 0 | 3,968 | 100 | 64 |
| recover | `recovered_held` | reopen the log | 3,952 [3,940–3,956] | 3,952 [3,940–3,956] | 0 | 2,140 [2,140–2,144] | 0 | 0 |
| recover | `read_one_held` | read one record | 3,952 [3,940–3,956] | 3,952 [3,940–3,956] | 0 | 2,140 [2,140–2,144] | 0 | 0 |
| recover | `read_one_dropped` | check and drop one record | 3,952 [3,940–3,956] | 3,952 [3,940–3,956] | 0 | 2,140 [2,140–2,144] | 0 | 0 |
| recover | `validation_held` | read every record | 7,144 [7,132–7,148] | 7,144 [7,132–7,148] | 0 | 5,332 [5,332–5,336] | 0 | 0 |
| recover | `validation_dropped` | check and drop every record | 5,732 [5,720–5,736] | 7,144 [7,132–7,148] | 0 | 5,332 [5,332–5,336] | 76 [76–200] | 72 [72–76] |
| recover | `engine_dropped` | drop the engine | 5,632 [5,620–5,636] | 5,732 [5,720–5,736] | 0 | 3,920 [3,920–3,924] | 76 | 64 |

### 1m

1,000,000 records, 0 keys, 1,000 streams, 1,000 records in each group, 32 payload bytes. KiB, median [range] of 3 processes in each cell; pinned and unpinned processes together.

| Mode | Barrier | Phase that ends there | Steady RSS | Exact phase peak | Peak above both ends | Peak above baseline | Mark low, unpinned | Mark low, pinned |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `touched_held` | allocate and fill one buffer | 33,064 [33,052–33,068] | 33,064 [33,052–33,068] | 0 | 31,252 | 0 | 0 |
| touch | `touched_dropped` | check and drop the buffer | 1,812 [1,800–1,816] | 33,064 [33,052–33,068] | 0 | 31,252 | 0 [0–64] | 64 [0–64] |
| caller | `caller_held` | build every submit | 165,918 [165,900–165,920] | 165,918 [165,900–165,920] | 0 | 164,104 | 0 | 0 |
| caller | `caller_dropped` | drop every submit | 165,918 [165,900–165,920] | 165,918 [165,900–165,920] | 0 | 164,104 | 0 | 0 |
| empty | `empty_held` | open a new log | 2,192 [2,192–2,196] | 2,192 [2,192–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_held` | read the empty log | 2,192 [2,192–2,196] | 2,192 [2,192–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_dropped` | drop the empty result | 2,192 [2,192–2,196] | 2,192 [2,192–2,196] | 0 | 380 | 0 | 0 |
| empty | `engine_dropped` | drop the engine | 2,092 [2,092–2,096] | 2,192 [2,192–2,196] | 0 | 380 | 100 | 100 |
| append | `empty_held` | open a new log | 2,190 [2,176–2,196] | 2,190 [2,176–2,196] | 0 | 380 | 0 | 0 |
| append | `appended_held` | append every group | 2,784 [2,768–2,788] | 2,784 [2,768–2,788] | 0 | 974 [972–976] | 0 | 0 |
| append | `read_one_held` | read one record | 2,784 [2,768–2,788] | 2,784 [2,768–2,788] | 0 | 974 [972–976] | 0 | 0 |
| append | `read_one_dropped` | check and drop one record | 2,784 [2,768–2,788] | 2,784 [2,768–2,788] | 0 | 974 [972–976] | 0 | 0 |
| append | `validation_held` | read every record | 167,694 [166,908–167,956] | 167,694 [166,908–167,956] | 0 | 165,886 [165,112–166,144] | 0 | 0 |
| append | `validation_dropped` | check and drop every record | 80,894 [80,880–80,900] | 167,694 [166,908–167,956] | 0 | 165,886 [165,112–166,144] | 204 [200–240] | 140 [88–144] |
| append | `engine_dropped` | drop the engine | 80,794 [80,780–80,800] | 80,894 [80,880–80,900] | 0 | 79,084 [79,084–79,088] | 100 [84–100] | 64 |
| recover | `recovered_held` | reopen the log | 2,512 [2,504–2,516] | 2,512 [2,504–2,516] | 0 | 700 | 0 | 0 |
| recover | `read_one_held` | read one record | 2,512 [2,504–2,516] | 2,512 [2,504–2,516] | 0 | 700 | 0 | 0 |
| recover | `read_one_dropped` | check and drop one record | 2,512 [2,504–2,516] | 2,512 [2,504–2,516] | 0 | 700 | 0 | 0 |
| recover | `validation_held` | read every record | 166,642 [166,632–167,668] | 166,642 [166,632–167,668] | 0 | 164,828 [164,824–165,856] | 0 | 0 |
| recover | `validation_dropped` | check and drop every record | 80,612 [80,604–80,616] | 166,642 [166,632–167,668] | 0 | 164,828 [164,824–165,856] | 192 [156–220] | 180 [92–184] |
| recover | `engine_dropped` | drop the engine | 80,512 [80,504–80,516] | 80,612 [80,604–80,616] | 0 | 78,800 | 76 [76–100] | 64 |

### 1m-keyed

1,000,000 records, 1,000,000 keys, 1,000 streams, 1,000 records in each group, 32 payload bytes. KiB, median [range] of 3 processes in each cell; pinned and unpinned processes together.

| Mode | Barrier | Phase that ends there | Steady RSS | Exact phase peak | Peak above both ends | Peak above baseline | Mark low, unpinned | Mark low, pinned |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `touched_held` | allocate and fill one buffer | 33,066 [33,060–33,068] | 33,066 [33,060–33,068] | 0 | 31,252 | 0 | 0 |
| touch | `touched_dropped` | check and drop the buffer | 1,814 [1,808–1,816] | 33,066 [33,060–33,068] | 0 | 31,252 | 0 [0–64] | 0 [0–64] |
| caller | `caller_held` | build every submit | 197,164 [197,152–197,164] | 197,164 [197,152–197,164] | 0 | 195,352 | 0 | 0 |
| caller | `caller_dropped` | drop every submit | 197,164 [197,152–197,164] | 197,164 [197,152–197,164] | 0 | 195,352 | 0 | 0 |
| empty | `empty_held` | open a new log | 2,194 [2,184–2,196] | 2,194 [2,184–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_held` | read the empty log | 2,194 [2,184–2,196] | 2,194 [2,184–2,196] | 0 | 380 | 0 | 0 |
| empty | `validation_dropped` | drop the empty result | 2,194 [2,184–2,196] | 2,194 [2,184–2,196] | 0 | 380 | 0 | 0 |
| empty | `engine_dropped` | drop the engine | 2,094 [2,084–2,096] | 2,194 [2,184–2,196] | 0 | 380 | 100 | 100 |
| append | `empty_held` | open a new log | 2,192 [2,184–2,196] | 2,192 [2,184–2,196] | 0 | 380 | 0 | 0 |
| append | `appended_held` | append every group | 103,326 [103,312–103,336] | 103,326 [103,312–103,336] | 0 | 101,514 [101,508–101,520] | 0 | 0 |
| append | `read_one_held` | read one record | 103,380 [103,368–103,388] | 103,380 [103,368–103,388] | 0 | 101,568 [101,564–101,572] | 0 | 0 |
| append | `read_one_dropped` | check and drop one record | 103,380 [103,368–103,388] | 103,380 [103,368–103,388] | 0 | 101,568 [101,564–101,572] | 0 | 0 |
| append | `validation_held` | read every record | 298,658 [298,648–298,668] | 298,658 [298,648–298,668] | 0 | 296,846 [296,840–296,852] | 0 | 0 |
| append | `validation_dropped` | check and drop every record | 212,636 [212,624–212,644] | 298,658 [298,648–298,668] | 0 | 296,846 [296,840–296,852] | 308 [248–328] | 132 [128–132] |
| append | `engine_dropped` | drop the engine | 212,522 [212,512–212,536] | 212,636 [212,624–212,644] | 0 | 210,824 [210,816–210,828] | 124 | 64 |
| recover | `recovered_held` | reopen the log | 102,988 [102,980–102,992] | 102,988 [102,980–102,992] | 0 | 101,176 [101,176–101,180] | 0 | 0 |
| recover | `read_one_held` | read one record | 102,988 [102,980–102,992] | 102,988 [102,980–102,992] | 0 | 101,176 [101,176–101,180] | 0 | 0 |
| recover | `read_one_dropped` | check and drop one record | 102,988 [102,980–102,992] | 102,988 [102,980–102,992] | 0 | 101,176 [101,176–101,180] | 0 | 0 |
| recover | `validation_held` | read every record | 298,344 [298,336–299,372] | 298,344 [298,336–299,372] | 0 | 296,532 [296,528–297,560] | 0 | 0 |
| recover | `validation_dropped` | check and drop every record | 212,316 [212,308–212,320] | 298,344 [298,336–299,372] | 0 | 296,532 [296,528–297,560] | 132 [112–140] | 120 [92–128] |
| recover | `engine_dropped` | drop the engine | 212,216 [212,208–212,220] | 212,316 [212,308–212,320] | 0 | 210,504 [210,504–210,508] | 76 | 64 [36–64] |

### pulse

16,384 records, 0 keys, 1 streams, 1 records in each group, 1,024 payload bytes. KiB, median [range] of 1 processes in each cell; pinned and unpinned processes together.

| Mode | Barrier | Phase that ends there | Steady RSS | Exact phase peak | Peak above both ends | Peak above baseline | Mark low, unpinned | Mark low, pinned |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| pulse | `pulsed` | allocate, fill, check and drop one buffer | 1,818 [1,816–1,820] | 18,206 [18,204–18,208] | 16,388 | 16,392 | 0 | 0 |

### Instrument

- 122 processes, 628 barriers and 506 phases. 1,513,507 events from the processes' own threads, 0 from other contexts.
- The replay ends at `VmRSS` at every barrier, and no instance lost an event: both are checks that refuse a process, and every process passed them.
- Unpinned: the mark is below the exact peak in 68 of 253 phases, by 64–376 KiB (median 100.0), and above it in 0.
- Pinned: the mark is below the exact peak in 69 of 253 phases, by 36–188 KiB (median 100), and above it in 0.
- Phases outside the pulse control that peak above both of their ends: 0 of 504.
- Barriers with transparent huge pages: 36.

## Engine-owned bytes

### 16k

KiB, median [range] of the processes in each cell. Engine bytes are requested bytes in the engine's arena; "held" leaves out what the caller holds of the engine's results.

| Mode | Barrier | Engine held | Engine peak in the phase | Results the caller holds | Caller and test live | Caller and test peak | Arena resident | Writer stack resident |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| append | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| append | `empty_held` | 289.7 | 289.7 | 0 | 1.8 | 1.8 | 296 | 12 |
| append | `caller_held` | 289.8 | 289.8 | 0 | 2,051.3 | 2,051.3 | 296 | 12 |
| append | `appended_held` | 292.8 | 327.5 | 0 | 1.8 | 2,051.3 | 340 | 12 |
| append | `read_one_held` | 292.8 | 321.2 | 0.4 | 1.8 | 1.8 | 340 | 12 |
| append | `read_one_dropped` | 292.8 | 293.2 | 0 | 1.8 | 1.8 | 340 | 12 |
| append | `validation_held` | 292.8 | 2,371.3 | 2,048 | 1.8 | 1.8 | 3,276 | 12 |
| append | `validation_dropped` | 292.8 | 2,340.8 | 0 | 1.8 | 1.8 | 3,276 | 12 |
| append | `engine_dropped` | 0 | 292.8 | 0 | 1.8 | 1.8 | 3,276 | 12 |
| recover | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| recover | `recovered_held` | 292.6 | 292.6 | 0 | 1.8 | 1.8 | 324 | 12 |
| recover | `read_one_held` | 292.7 | 321.2 | 0.4 | 1.8 | 1.8 | 352 | 12 |
| recover | `read_one_dropped` | 292.7 | 293.1 | 0 | 1.8 | 1.8 | 352 | 12 |
| recover | `validation_held` | 292.7 | 2,371.3 | 2,048 | 1.8 | 1.8 | 3,276 | 12 |
| recover | `validation_dropped` | 292.7 | 2,340.7 | 0 | 1.8 | 1.8 | 3,276 | 12 |
| recover | `engine_dropped` | 0 | 292.7 | 0 | 1.8 | 1.8 | 3,276 | 12 |

### 16k-keyed

KiB, median [range] of the processes in each cell. Engine bytes are requested bytes in the engine's arena; "held" leaves out what the caller holds of the engine's results.

| Mode | Barrier | Engine held | Engine peak in the phase | Results the caller holds | Caller and test live | Caller and test peak | Arena resident | Writer stack resident |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| append | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| append | `empty_held` | 289.7 | 289.7 | 0 | 1.8 | 1.8 | 296 | 12 |
| append | `caller_held` | 289.9 | 289.9 | 0 | 2,179.3 | 2,179.3 | 296 | 12 |
| append | `appended_held` | 1,603.1 | 1,639.9 | 0 | 1.8 | 2,179.3 | 2,096 | 12 |
| append | `read_one_held` | 1,603.1 | 1,633.6 | 0.4 | 1.8 | 1.8 | 2,096 | 12 |
| append | `read_one_dropped` | 1,603.1 | 1,603.5 | 0 | 1.8 | 1.8 | 2,096 | 12 |
| append | `validation_held` | 1,603.1 | 3,811.7 | 2,176 | 1.8 | 1.8 | 5,544 | 12 |
| append | `validation_dropped` | 1,603.1 | 3,779.1 | 0 | 1.8 | 1.8 | 5,544 | 12 |
| append | `engine_dropped` | 0 | 1,603.1 | 0 | 1.8 | 1.8 | 5,544 | 12 |
| recover | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| recover | `recovered_held` | 1,603.0 | 1,603.0 | 0 | 1.8 | 1.8 | 2,100 | 12 |
| recover | `read_one_held` | 1,603.1 | 1,633.5 | 0.4 | 1.8 | 1.8 | 2,116 | 12 |
| recover | `read_one_dropped` | 1,603.1 | 1,603.5 | 0 | 1.8 | 1.8 | 2,116 | 12 |
| recover | `validation_held` | 1,603.1 | 3,811.6 | 2,176 | 1.8 | 1.8 | 5,544 | 12 |
| recover | `validation_dropped` | 1,603.1 | 3,779.1 | 0 | 1.8 | 1.8 | 5,544 | 12 |
| recover | `engine_dropped` | 0 | 1,603.1 | 0 | 1.8 | 1.8 | 5,544 | 12 |

### 1m

KiB, median [range] of the processes in each cell. Engine bytes are requested bytes in the engine's arena; "held" leaves out what the caller holds of the engine's results.

| Mode | Barrier | Engine held | Engine peak in the phase | Results the caller holds | Caller and test live | Caller and test peak | Arena resident | Writer stack resident |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| append | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| append | `empty_held` | 289.7 | 289.7 | 0 | 1.8 | 1.8 | 296 | 12 |
| append | `caller_held` | 289.8 | 289.8 | 0 | 125,025.2 | 125,025.2 | 296 | 12 |
| append | `appended_held` | 439.2 [433.3–442.9] | 614.3 | 0 | 1.8 | 125,025.2 | 732 | 12 |
| append | `read_one_held` | 412.5 | 522.3 | 0.4 | 1.8 | 1.8 | 732 | 12 |
| append | `read_one_dropped` | 412.5 | 412.8 | 0 | 1.8 | 1.8 | 732 | 12 |
| append | `validation_held` | 412.5 | 129,735.4 | 129,174.5 | 1.8 | 1.8 | 180,744 | 12 |
| append | `validation_dropped` | 412.5 | 129,587.0 | 0 | 1.8 | 1.8 | 180,744 | 12 |
| append | `engine_dropped` | 0 | 412.5 | 0 | 1.8 | 1.8 | 180,744 | 12 |
| recover | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| recover | `recovered_held` | 412.3 | 412.3 | 0 | 1.8 | 1.8 | 516 | 12 |
| recover | `read_one_held` | 412.4 | 522.2 | 0.4 | 1.8 | 1.8 | 572 | 12 |
| recover | `read_one_dropped` | 412.4 | 412.8 | 0 | 1.8 | 1.8 | 572 | 12 |
| recover | `validation_held` | 412.4 | 129,735.4 | 129,174.5 | 1.8 | 1.8 | 180,744 | 12 |
| recover | `validation_dropped` | 412.4 | 129,586.9 | 0 | 1.8 | 1.8 | 180,744 | 12 |
| recover | `engine_dropped` | 0 | 412.4 | 0 | 1.8 | 1.8 | 180,744 | 12 |

### 1m-keyed

KiB, median [range] of the processes in each cell. Engine bytes are requested bytes in the engine's arena; "held" leaves out what the caller holds of the engine's results.

| Mode | Barrier | Engine held | Engine peak in the phase | Results the caller holds | Caller and test live | Caller and test peak | Arena resident | Writer stack resident |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| append | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| append | `empty_held` | 289.7 | 289.7 | 0 | 1.8 | 1.8 | 296 | 12 |
| append | `caller_held` | 289.9 | 289.9 | 0 | 132,837.7 | 132,837.7 | 296 | 12 |
| append | `appended_held` | 76,266.2 [76,252.1–76,284.7] | 76,461.8 | 0 | 1.8 | 132,837.8 | 103,468 | 12 |
| append | `read_one_held` | 76,252.1 | 76,369.8 | 0.4 | 1.8 | 1.8 | 103,468 | 12 |
| append | `read_one_dropped` | 76,252.1 | 76,252.5 | 0 | 1.8 | 1.8 | 103,468 | 12 |
| append | `validation_held` | 76,252.1 | 213,395.4 | 136,987 | 1.8 | 1.8 | 314,312 | 12 |
| append | `validation_dropped` | 76,252.1 | 213,239.1 | 0 | 1.8 | 1.8 | 314,312 | 12 |
| append | `engine_dropped` | 0 | 76,252.1 | 0 | 1.8 | 1.8 | 314,312 | 12 |
| recover | `baseline` | 0 | 0 | 0 | 1.8 | 1.8 | 8 | 0 |
| recover | `recovered_held` | 76,251.9 | 76,251.9 | 0 | 1.8 | 1.8 | 103,800 | 12 |
| recover | `read_one_held` | 76,252.1 | 76,369.7 | 0.4 | 1.8 | 1.8 | 103,920 | 12 |
| recover | `read_one_dropped` | 76,252.1 | 76,252.5 | 0 | 1.8 | 1.8 | 103,920 | 12 |
| recover | `validation_held` | 76,252.1 | 213,395.3 | 136,987 | 1.8 | 1.8 | 314,312 | 12 |
| recover | `validation_dropped` | 76,252.1 | 213,239.1 | 0 | 1.8 | 1.8 | 314,312 | 12 |
| recover | `engine_dropped` | 0 | 76,252.1 | 0 | 1.8 | 1.8 | 314,312 | 12 |

