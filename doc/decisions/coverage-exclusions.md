# Coverage 제외 규약

## 결정

저장소가 소유한 Rust source의 coverage 제외는 파일 안의 LCOV marker로만 표현한다. Rust nightly의 `coverage(off)` attribute와 `llvm-cov --ignore-filename-regex`는 사용하지 않는다. 파일을 열었을 때 제외 여부와 사유를 바로 확인할 수 있어야 하기 때문이다.

허용 문법은 다음과 같다.

```rust
fallible_boundary()?; // LCOV_EXCL_LINE -- rustc가 실행 불가능한 별도 region을 만든다.

// LCOV_EXCL_START -- 실제 OS window가 필요한 경계이며 headed E2E로 검증한다.
platform_call();
present_to_window();
// LCOV_EXCL_STOP
```

파일 전체 제외는 첫 번째 비어 있지 않은 줄에만 둔다.

```rust
// LCOV_EXCL_FILE -- 생성된 platform glue이며 browser E2E로 검증한다.
```

`LCOV_EXCL_LINE`, `LCOV_EXCL_START`, `LCOV_EXCL_STOP`은 LCOV의 source exclusion marker다. `LCOV_EXCL_FILE`은 이 프로젝트가 `lcov_filter`에 추가한 확장이며 해당 source의 `SF: ... end_of_record` 전체를 filtered LCOV에서 제거한다.

## 제약

- `LINE`, `START`, `FILE` marker에는 같은 줄의 `-- <사유>`가 필수다.
- `START`와 `STOP`은 독립된 주석으로 쓰고 중첩, 고아 `STOP`, 닫히지 않은 section을 허용하지 않는다.
- section 안에 `LINE`을 중복해서 쓰지 않는다.
- `FILE`은 파일당 한 번만 쓰며 line/section marker와 섞지 않는다.
- 제외 우선순위는 테스트 추가, `LINE`, 최소 `START`/`STOP`, `FILE` 순이다.
- `FILE`은 toolchain 생성 코드처럼 저장소에서 직접 실행하기 어려운 경계에만 사용하고 사전에 영향 범위와 대체 검증을 보고해 승인을 받는다.

## 검증

`pnpm run check:coverage-policy`가 `cargo metadata`의 workspace crate를 기준으로 각 package directory 아래의 소유 `.rs` 파일을 재귀 검사한다. custom lib/bin/build-script 경로도 포함하며 `.git/`과 Cargo 산출물인 `target/`만 제외한다. `coverage(off)`, 잘못된 marker 위치, 사유 누락과 section 불균형은 정적 검사에서 실패한다.

`pnpm run coverage`는 `cargo +nightly llvm-cov --lcov` 결과를 `lcov_filter --text`로 후처리한다. `pipefail`을 유지하며 다음을 출력한다.

```text
Included Lines: 123/123
Excluded Lines: 2
Excluded Files (1)
path/to/platform.rs: 실제 OS window가 필요한 경계다.
Missing Lines (0)
```

missing line이 하나라도 있거나 제외 뒤 포함 line이 0이면 실패한다. 완료 보고에는 included/excluded line 수, 제외 파일과 사유, `Missing Lines (0)`, 제외한 코드의 대체 unit/E2E 증거를 포함한다.

## 참고

- [LCOV exclusion markers](https://github.com/linux-test-project/lcov/blob/master/docs/man/geninfo.rst#exclusion-markers)
- [lcov_filter](https://github.com/cLazyZombie/lcov_filter)
