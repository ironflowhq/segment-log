61 processes, 0 failed.

### 16k

16,384 records, 0 keys, 1 streams, 256 records in each group, 32 payload bytes. KiB, median [range] of 3 processes for each mode.

| Mode | Barrier | Phase that ends there | Steady RSS | Phase peak | Anonymous | File-backed | Steady above baseline | Peak above baseline |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| touch | `touched_held` | allocate and fill one buffer | 2,332 [2,332–2,336] | 2,332 [2,332–2,336] | 636 [636–640] | 1,696 | 520 | 520 |
| touch | `touched_dropped` | check and drop the buffer | 1,816 [1,816–1,820] | 2,332 [2,332–2,336] | 120 [120–124] | 1,696 | 4 | 520 |
| caller | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| caller | `caller_held` | build every submit | 4,504 [4,504–4,508] | 4,504 [4,504–4,508] | 2,808 [2,808–2,812] | 1,696 | 2,692 | 2,692 |
| caller | `caller_dropped` | drop every submit | 4,504 [4,504–4,508] | 4,504 [4,504–4,508] | 2,808 [2,808–2,812] | 1,696 | 2,692 | 2,692 |
| empty | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| empty | `empty_held` | open a new log | 2,128 [2,120–2,128] | 2,128 [2,120–2,128] | 368 [360–368] | 1,760 | 316 [304–316] | 316 [304–316] |
| empty | `validation_held` | read the empty log | 2,128 [2,128–2,132] | 2,128 [2,128–2,132] | 368 [368–372] | 1,760 | 316 | 316 |
| empty | `validation_dropped` | drop the empty result | 2,128 [2,128–2,132] | 2,128 [2,128–2,132] | 368 [368–372] | 1,760 | 316 | 316 |
| empty | `engine_dropped` | drop the engine | 2,092 [2,092–2,096] | 2,128 [2,128–2,132] | 140 [140–144] | 1,952 | 280 | 316 |
| append | `baseline` | start the process | 1,812 [1,800–1,812] | 1,812 [1,800–1,812] | 116 | 1,696 [1,684–1,696] | 0 | 0 |
| append | `empty_held` | open a new log | 2,128 [2,116–2,128] | 2,128 [2,116–2,128] | 368 | 1,760 [1,748–1,760] | 316 | 316 |
| append | `appended_held` | append every group | 2,212 [2,200–2,212] | 2,212 [2,200–2,212] | 452 | 1,760 [1,748–1,760] | 400 | 400 |
| append | `read_one_held` | read one record | 2,224 [2,212–2,224] | 2,224 [2,212–2,224] | 464 | 1,760 [1,748–1,760] | 412 | 412 |
| append | `read_one_dropped` | check and drop one record | 2,224 [2,212–2,224] | 2,224 [2,212–2,224] | 464 | 1,760 [1,748–1,760] | 412 | 412 |
| append | `validation_held` | read every record | 4,892 [4,880–4,892] | 4,892 [4,880–4,892] | 3,132 | 1,760 [1,748–1,760] | 3,080 | 3,080 |
| append | `validation_dropped` | check and drop every record | 3,480 [3,468–3,480] | 4,892 [4,880–4,892] | 1,720 | 1,760 [1,748–1,760] | 1,668 | 3,080 |
| append | `engine_dropped` | drop the engine | 3,444 [3,432–3,444] | 3,480 [3,468–3,480] | 1,492 | 1,952 [1,940–1,952] | 1,632 | 1,668 |
| recover | `baseline` | start the process | 1,812 | 1,812 | 116 | 1,696 | 0 | 0 |
| recover | `recovered_held` | reopen the log | 2,168 [2,164–2,172] | 2,168 [2,164–2,172] | 408 [404–412] | 1,760 | 356 [352–360] | 356 [352–360] |
| recover | `read_one_held` | read one record | 2,172 | 2,172 | 412 | 1,760 | 360 | 360 |
| recover | `read_one_dropped` | check and drop one record | 2,172 | 2,172 | 412 | 1,760 | 360 | 360 |
| recover | `validation_held` | read every record | 4,852 | 4,852 | 3,092 | 1,760 | 3,040 | 3,040 |
| recover | `validation_dropped` | check and drop every record | 3,440 | 4,852 | 1,680 | 1,760 | 1,628 | 3,040 |
| recover | `engine_dropped` | drop the engine | 3,404 | 3,440 | 1,452 | 1,952 | 1,592 | 1,628 |

### 16k-keyed

16,384 records, 16,384 keys, 1 streams, 256 records in each group, 32 payload bytes. KiB, median [range] of 3 processes for each mode.

| Mode | Barrier | Phase that ends there | Steady RSS | Phase peak | Anonymous | File-backed | Steady above baseline | Peak above baseline |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `baseline` | start the process | 1,796 [1,796–1,812] | 1,796 [1,796–1,812] | 116 | 1,680 [1,680–1,696] | 0 | 0 |
| touch | `touched_held` | allocate and fill one buffer | 2,312 [2,312–2,328] | 2,312 [2,312–2,328] | 632 | 1,680 [1,680–1,696] | 516 | 516 |
| touch | `touched_dropped` | check and drop the buffer | 1,796 [1,796–1,812] | 2,312 [2,312–2,328] | 116 | 1,680 [1,680–1,696] | 0 | 516 |
| caller | `baseline` | start the process | 1,812 [1,804–1,812] | 1,812 [1,804–1,812] | 116 | 1,696 [1,688–1,696] | 0 | 0 |
| caller | `caller_held` | build every submit | 5,016 [5,008–5,016] | 5,016 [5,008–5,016] | 3,320 | 1,696 [1,688–1,696] | 3,204 | 3,204 |
| caller | `caller_dropped` | drop every submit | 5,016 [5,008–5,016] | 5,016 [5,008–5,016] | 3,320 | 1,696 [1,688–1,696] | 3,204 | 3,204 |
| empty | `baseline` | start the process | 1,812 [1,800–1,812] | 1,812 [1,800–1,812] | 116 | 1,696 [1,684–1,696] | 0 | 0 |
| empty | `empty_held` | open a new log | 2,128 [2,116–2,128] | 2,128 [2,116–2,128] | 368 | 1,760 [1,748–1,760] | 316 | 316 |
| empty | `validation_held` | read the empty log | 2,128 [2,116–2,128] | 2,128 [2,116–2,128] | 368 | 1,760 [1,748–1,760] | 316 | 316 |
| empty | `validation_dropped` | drop the empty result | 2,128 [2,116–2,128] | 2,128 [2,116–2,128] | 368 | 1,760 [1,748–1,760] | 316 | 316 |
| empty | `engine_dropped` | drop the engine | 2,092 [2,080–2,092] | 2,128 [2,116–2,128] | 140 | 1,952 [1,940–1,952] | 280 | 316 |
| append | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| append | `empty_held` | open a new log | 2,128 [2,128–2,132] | 2,128 [2,128–2,132] | 368 [368–372] | 1,760 | 316 | 316 |
| append | `appended_held` | append every group | 3,944 [3,944–3,948] | 3,944 [3,944–3,948] | 2,184 [2,184–2,188] | 1,760 | 2,132 | 2,132 |
| append | `read_one_held` | read one record | 3,944 [3,944–3,948] | 3,944 [3,944–3,948] | 2,184 [2,184–2,188] | 1,760 | 2,132 | 2,132 |
| append | `read_one_dropped` | check and drop one record | 3,944 [3,944–3,948] | 3,944 [3,944–3,948] | 2,184 [2,184–2,188] | 1,760 | 2,132 | 2,132 |
| append | `validation_held` | read every record | 7,128 [7,128–7,132] | 7,128 [7,128–7,132] | 5,368 [5,368–5,372] | 1,760 | 5,316 | 5,316 |
| append | `validation_dropped` | check and drop every record | 5,716 [5,716–5,720] | 7,128 [7,128–7,132] | 3,956 [3,956–3,960] | 1,760 | 3,904 | 5,316 |
| append | `engine_dropped` | drop the engine | 5,680 [5,680–5,684] | 5,716 [5,716–5,720] | 3,728 [3,728–3,732] | 1,952 | 3,868 | 3,904 |
| recover | `baseline` | start the process | 1,812 [1,796–1,812] | 1,812 [1,796–1,812] | 116 | 1,696 [1,680–1,696] | 0 | 0 |
| recover | `recovered_held` | reopen the log | 3,888 [3,872–3,888] | 3,888 [3,872–3,888] | 2,128 | 1,760 [1,744–1,760] | 2,076 | 2,076 |
| recover | `read_one_held` | read one record | 3,888 [3,872–3,888] | 3,888 [3,872–3,888] | 2,128 | 1,760 [1,744–1,760] | 2,076 | 2,076 |
| recover | `read_one_dropped` | check and drop one record | 3,888 [3,872–3,888] | 3,888 [3,872–3,888] | 2,128 | 1,760 [1,744–1,760] | 2,076 | 2,076 |
| recover | `validation_held` | read every record | 7,080 [7,064–7,080] | 7,080 [7,064–7,080] | 5,320 | 1,760 [1,744–1,760] | 5,268 | 5,268 |
| recover | `validation_dropped` | check and drop every record | 5,668 [5,652–5,668] | 7,080 [7,064–7,080] | 3,908 | 1,760 [1,744–1,760] | 3,856 | 5,268 |
| recover | `engine_dropped` | drop the engine | 5,632 [5,616–5,632] | 5,668 [5,652–5,668] | 3,680 | 1,952 [1,936–1,952] | 3,820 | 3,856 |

### 1m

1,000,000 records, 0 keys, 1,000 streams, 1,000 records in each group, 32 payload bytes. KiB, median [range] of 3 processes for each mode.

| Mode | Barrier | Phase that ends there | Steady RSS | Phase peak | Anonymous | File-backed | Steady above baseline | Peak above baseline |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `baseline` | start the process | 1,812 [1,800–1,816] | 1,812 [1,800–1,816] | 116 [116–120] | 1,696 [1,684–1,696] | 0 | 0 |
| touch | `touched_held` | allocate and fill one buffer | 33,068 [33,056–33,072] | 33,068 [33,056–33,072] | 31,372 [31,372–31,376] | 1,696 [1,684–1,696] | 31,256 | 31,256 |
| touch | `touched_dropped` | check and drop the buffer | 1,816 [1,804–1,820] | 33,068 [33,056–33,072] | 120 [120–124] | 1,696 [1,684–1,696] | 4 | 31,256 |
| caller | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| caller | `caller_held` | build every submit | 165,916 [165,916–165,920] | 165,916 [165,916–165,920] | 164,220 [164,220–164,224] | 1,696 | 164,104 | 164,104 |
| caller | `caller_dropped` | drop every submit | 165,916 [165,916–165,920] | 165,916 [165,916–165,920] | 164,220 [164,220–164,224] | 1,696 | 164,104 | 164,104 |
| empty | `baseline` | start the process | 1,812 [1,808–1,812] | 1,812 [1,808–1,812] | 116 [116–120] | 1,696 [1,688–1,696] | 0 | 0 |
| empty | `empty_held` | open a new log | 2,128 [2,116–2,128] | 2,128 [2,116–2,128] | 368 [364–368] | 1,760 [1,752–1,760] | 316 [308–316] | 316 [308–316] |
| empty | `validation_held` | read the empty log | 2,128 [2,124–2,128] | 2,128 [2,124–2,128] | 368 [368–372] | 1,760 [1,752–1,760] | 316 | 316 |
| empty | `validation_dropped` | drop the empty result | 2,128 [2,124–2,128] | 2,128 [2,124–2,128] | 368 [368–372] | 1,760 [1,752–1,760] | 316 | 316 |
| empty | `engine_dropped` | drop the engine | 2,092 [2,088–2,092] | 2,128 [2,124–2,128] | 140 [140–144] | 1,952 [1,944–1,952] | 280 | 316 |
| append | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| append | `empty_held` | open a new log | 2,128 [2,120–2,128] | 2,128 [2,120–2,128] | 368 [360–368] | 1,760 | 316 [304–316] | 316 [304–316] |
| append | `appended_held` | append every group | 2,720 [2,720–2,724] | 2,720 [2,720–2,724] | 960 [960–964] | 1,760 | 908 | 908 |
| append | `read_one_held` | read one record | 2,720 [2,720–2,724] | 2,720 [2,720–2,724] | 960 [960–964] | 1,760 | 908 | 908 |
| append | `read_one_dropped` | check and drop one record | 2,720 [2,720–2,724] | 2,720 [2,720–2,724] | 960 [960–964] | 1,760 | 908 | 908 |
| append | `validation_held` | read every record | 166,860 [166,856–166,860] | 166,860 [166,856–166,860] | 165,100 [165,096–165,100] | 1,760 | 165,044 [165,044–165,048] | 165,044 [165,044–165,048] |
| append | `validation_dropped` | check and drop every record | 80,832 [80,832–80,836] | 166,860 [166,856–166,860] | 79,072 [79,072–79,076] | 1,760 | 79,020 | 165,044 [165,044–165,048] |
| append | `engine_dropped` | drop the engine | 80,796 [80,796–80,800] | 80,832 [80,832–80,836] | 78,844 [78,844–78,848] | 1,952 | 78,984 | 79,020 |
| recover | `baseline` | start the process | 1,812 [1,800–1,816] | 1,812 [1,800–1,816] | 120 [116–120] | 1,696 [1,680–1,696] | 0 | 0 |
| recover | `recovered_held` | reopen the log | 2,436 [2,436–2,452] | 2,436 [2,436–2,452] | 692 [676–692] | 1,760 [1,744–1,760] | 636 [624–636] | 636 [624–636] |
| recover | `read_one_held` | read one record | 2,448 [2,436–2,452] | 2,448 [2,436–2,452] | 692 [688–692] | 1,760 [1,744–1,760] | 636 | 636 |
| recover | `read_one_dropped` | check and drop one record | 2,448 [2,436–2,452] | 2,448 [2,436–2,452] | 692 [688–692] | 1,760 [1,744–1,760] | 636 | 636 |
| recover | `validation_held` | read every record | 166,580 [166,576–167,592] | 166,580 [166,576–167,592] | 164,820 [164,816–165,848] | 1,760 [1,744–1,760] | 164,764 [164,764–165,792] | 164,764 [164,764–165,792] |
| recover | `validation_dropped` | check and drop every record | 80,548 [80,536–80,552] | 166,580 [166,576–167,592] | 78,792 [78,788–78,792] | 1,760 [1,744–1,760] | 78,736 | 164,764 [164,764–165,792] |
| recover | `engine_dropped` | drop the engine | 80,512 [80,500–80,516] | 80,548 [80,536–80,552] | 78,564 [78,560–78,564] | 1,952 [1,936–1,952] | 78,700 | 78,736 |

### 1m-keyed

1,000,000 records, 1,000,000 keys, 1,000 streams, 1,000 records in each group, 32 payload bytes. KiB, median [range] of 3 processes for each mode.

| Mode | Barrier | Phase that ends there | Steady RSS | Phase peak | Anonymous | File-backed | Steady above baseline | Peak above baseline |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| touch | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| touch | `touched_held` | allocate and fill one buffer | 33,064 [33,064–33,068] | 33,064 [33,064–33,068] | 31,368 [31,368–31,372] | 1,696 | 31,252 | 31,252 |
| touch | `touched_dropped` | check and drop the buffer | 1,812 [1,812–1,816] | 33,064 [33,064–33,068] | 116 [116–120] | 1,696 | 0 | 31,252 |
| caller | `baseline` | start the process | 1,816 [1,812–1,816] | 1,816 [1,812–1,816] | 120 [116–120] | 1,696 | 0 | 0 |
| caller | `caller_held` | build every submit | 197,168 [197,164–197,168] | 197,168 [197,164–197,168] | 195,472 [195,468–195,472] | 1,696 | 195,352 | 195,352 |
| caller | `caller_dropped` | drop every submit | 197,168 [197,164–197,168] | 197,168 [197,164–197,168] | 195,472 [195,468–195,472] | 1,696 | 195,352 | 195,352 |
| empty | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| empty | `empty_held` | open a new log | 2,128 [2,116–2,132] | 2,128 [2,116–2,132] | 368 [356–372] | 1,760 | 316 [304–316] | 316 [304–316] |
| empty | `validation_held` | read the empty log | 2,128 [2,128–2,132] | 2,128 [2,128–2,132] | 368 [368–372] | 1,760 | 316 | 316 |
| empty | `validation_dropped` | drop the empty result | 2,128 [2,128–2,132] | 2,128 [2,128–2,132] | 368 [368–372] | 1,760 | 316 | 316 |
| empty | `engine_dropped` | drop the engine | 2,092 [2,092–2,096] | 2,128 [2,128–2,132] | 140 [140–144] | 1,952 | 280 | 316 |
| append | `baseline` | start the process | 1,816 [1,812–1,816] | 1,816 [1,812–1,816] | 120 [116–120] | 1,696 | 0 | 0 |
| append | `empty_held` | open a new log | 2,128 [2,124–2,132] | 2,128 [2,124–2,132] | 368 [364–372] | 1,760 | 316 [308–316] | 316 [308–316] |
| append | `appended_held` | append every group | 103,244 [103,244–103,248] | 103,244 [103,244–103,248] | 101,484 [101,484–101,488] | 1,760 | 101,432 [101,428–101,432] | 101,432 [101,428–101,432] |
| append | `read_one_held` | read one record | 103,300 [103,296–103,300] | 103,300 [103,296–103,300] | 101,540 [101,536–101,540] | 1,760 | 101,484 | 101,484 |
| append | `read_one_dropped` | check and drop one record | 103,300 [103,296–103,300] | 103,300 [103,296–103,300] | 101,540 [101,536–101,540] | 1,760 | 101,484 | 101,484 |
| append | `validation_held` | read every record | 298,576 [298,576–299,240] | 298,576 [298,576–299,240] | 296,816 [296,816–297,480] | 1,760 | 296,764 [296,760–297,424] | 296,764 [296,760–297,424] |
| append | `validation_dropped` | check and drop every record | 212,556 [212,552–212,556] | 298,576 [298,576–299,240] | 210,796 [210,792–210,796] | 1,760 | 210,740 | 296,764 [296,760–297,424] |
| append | `engine_dropped` | drop the engine | 212,520 [212,516–212,520] | 212,556 [212,552–212,556] | 210,568 [210,564–210,568] | 1,952 | 210,704 | 210,740 |
| recover | `baseline` | start the process | 1,812 [1,812–1,816] | 1,812 [1,812–1,816] | 116 [116–120] | 1,696 | 0 | 0 |
| recover | `recovered_held` | reopen the log | 102,924 [102,916–102,924] | 102,924 [102,916–102,924] | 101,164 [101,156–101,164] | 1,760 | 101,112 [101,100–101,112] | 101,112 [101,100–101,112] |
| recover | `read_one_held` | read one record | 102,924 [102,924–102,928] | 102,924 [102,924–102,928] | 101,164 [101,164–101,168] | 1,760 | 101,112 | 101,112 |
| recover | `read_one_dropped` | check and drop one record | 102,924 [102,924–102,928] | 102,924 [102,924–102,928] | 101,164 [101,164–101,168] | 1,760 | 101,112 | 101,112 |
| recover | `validation_held` | read every record | 298,280 [298,276–298,280] | 298,280 [298,276–298,280] | 296,520 [296,516–296,520] | 1,760 | 296,464 [296,464–296,468] | 296,464 [296,464–296,468] |
| recover | `validation_dropped` | check and drop every record | 212,252 [212,252–212,256] | 298,280 [298,276–298,280] | 210,492 [210,492–210,496] | 1,760 | 210,440 | 296,464 [296,464–296,468] |
| recover | `engine_dropped` | drop the engine | 212,216 [212,216–212,220] | 212,252 [212,252–212,256] | 210,264 [210,264–210,268] | 1,952 | 210,404 | 210,440 |

### Instrument

- 315 barriers in 61 processes, the reset check included.
- `VmRSS` differs from the `Rss` of `smaps_rollup`: 4 of 315; the largest difference is 8 KiB.
- `RssAnon` differs from the `Anonymous` of `smaps_rollup`: 4 of 315; the largest difference is 8 KiB.
- Barriers with swapped pages: 0 of 315.
- Barriers with shared-memory pages: 0 of 315.
- Barriers with transparent huge pages: 19 of 315; the largest difference is 63,488 KiB.
- Resets that changed `VmRSS`: 5 of 315; the largest difference is 12 KiB.
- Resets that left the mark above `VmRSS`: 0 of 315.
- Phases whose mark is below the resident set at their start: 67 of 254; the largest difference is 576 KiB.
- Touched buffers whose resident growth differs from their size: 13 of 13; the largest difference is 8 KiB.
