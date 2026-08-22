# 부록 C. 수학과 알고리즘 빠른 참조

## 좌표/행렬

```text
열벡터: p_clip = P * V * M * p_object
왼손 view: +X right, +Y up, 카메라는 +Z를 본다.
F=normalize(target-eye), R=normalize(cross(world_up,F)), U=cross(F,R).
w_clip=z_view. P_lh_zo의 z rows는 [0,0,f/(f-n),-fn/(f-n)], [0,0,1,0].
z_view=near -> z_ndc=0, z_view=far -> z_ndc=1.
clip: -w<=x<=w, -w<=y<=w, 0<=z<=w.
NDC: clip/w. screen y = (0.5 - 0.5*y_ndc)*H.
```

## 클리핑

```text
plane distance: x+w, w-x, y+w, w-y, z, w-z.
교점 t=dA/(dA-dB), 모든 ClipVertex 속성을 같은 t로 lerp.
여섯 plane 후 polygon을 triangle fan으로 변환.
```

## 래스터

```text
E(a,b,p)=cross2(b-a,p-a). positive screen winding만 입력.
sample center=(x+0.5,y+0.5).
y-down top-left: dy<0 또는 dy=0, dx>0.
edge step x=-dy*S, step y=dx*S.
```

## 보간/깊이

```text
λ0=e0/area, λ1=e1/area, λ2=e2/area.
z_ndc는 screen affine, 작은 값이 가깝고 depth clear는 +infinity.
일반 속성 a=(Σλ*a/w)/(Σλ/w).
```

## 텍스처/색

```text
repeat(u)=u-floor(u). bilinear x=uW-0.5.
base color texel은 sRGB decode 후 filter/lighting.
lighting/blend/resolve는 linear, framebuffer 쓰기 직전 sRGB encode.
```

## 용어 사전

**Sample.** 픽셀 셀 안에서 삼각형 포함 여부와 깊이를 평가하는 위치.

**Coverage.** 한 primitive가 sample을 덮는지에 대한 결과.

**Clip space.** projection 뒤, w로 나누기 전의 동차 Vec4 공간.

**NDC.** clip/w 뒤 표준화된 좌표.

**Winding.** 삼각형 정점 순서가 만드는 signed orientation.

**Handedness.** 이 교재는 통상적인 cross 식을 유지하고, LH look-at의 cross 인자 순서와 projection의 `w_clip=z_view`로 좌표 규약을 고정한다.

**Barycentric.** 삼각형 내부점을 세 정점의 가중치로 표현한 좌표.

**Depth test.** 후보 표면이 현재 저장된 표면보다 가까운지 비교하는 단계.

**Perspective-correct.** 1/w를 이용해 투영 전 속성의 선형 관계를 복원하는 보간.

**Texel.** texture 이미지의 한 요소. 화면 pixel과 구분.

**Mipmap.** 축소 샘플링을 위해 미리 만든 여러 해상도의 texture chain.
