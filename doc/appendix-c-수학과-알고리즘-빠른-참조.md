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

## 공식을 읽는 기호와 상세 유도

삼각형 식의 Σ는 i=0,1,2의 세 항을 더한다. `λ`는 화면 가중치, `β`는 투영 전 표면 가중치다. 둘 다 합은 1이다.

```text
normalize(v)=v/sqrt(dot(v,v))      # 길이가 유효할 때만
q=Σ(λi/wi), βi=(λi/wi)/q
a=(Σλi*a_i/wi)/q
z_ndc=Σλi*z_ndc_i                # 다시 q로 나누지 않는다
```

유도와 수치 계산은 [14장 원근 보정](14-perspective-correct-보간.md)에 있다. 행렬 곱과 T/S/R 전체 행렬은 [5장](05-벡터와-행렬을-필요한-만큼만.md), projection 계수의 유도는 [7장](07-카메라-원근-투영-동차좌표-w.md), clip 교점의 유도는 [10장](10-동차-clip-공간에서-삼각형-자르기.md)을 따른다.

```text
bilinear: x=uW-0.5, y=vH-0.5
x0=floor(x), y0=floor(y), fx=x-x0, fy=y-y0
c=(1-fx)(1-fy)c00+fx(1-fy)c10+(1-fx)fy*c01+fx*fy*c11
```

네 이웃의 주소화와 linear 색 전제는 [17장](17-uv-주소화-nearest-bilinear-샘플링.md)에 있다.

## 조명·색·합성·품질

```text
normal_world=normalize(transpose(inverse(M3))*normal_object)
Lambert=max(dot(N,L),0)
H=normalize(L+V), specular=max(dot(N,H),0)^shininess  # N·L>0일 때
sRGB decode(c)=c/12.92 또는 ((c+0.055)/1.055)^2.4     # 경계 0.04045
sRGB encode(l)=12.92*l 또는 1.055*l^(1/2.4)-0.055    # 경계 0.0031308
Aout=As+Ad*(1-As)
Cout=(Cs*As+Cd*Ad*(1-As))/Aout                     # straight, Aout>0
Ad=1이면 Cout=Cs*As+Cd*(1-As)
rho=max(length(dUVdx*(W,H)),length(dUVdy*(W,H)))
lod=log2(max(rho,epsilon)), nearest_level=clamp(round(lod),0,last)
SSAA 2x resolve=(c00+c10+c01+c11)/4                 # linear 색
```

법선의 수직 조건과 inverse-transpose는 [18장](18-법선-변환과-lambert-조명.md), byte→linear→byte는 [19장](19-blinn-phong과-srgb-linear-색-공간.md), alpha 전제와 정렬 예제는 [22장](22-투명도-alpha-test-blending-순서.md), LOD의 단위와 SSAA는 [23장](23-antialiasing과-mipmap.md)을 따른다.

## 시간 통계와 animation

```text
nearest-rank percentile=sorted[ceil(p*N)-1]
Amdahl speedup=1/(s+(1-s)/N)
Δ=t1-t0, u=(t-t0)/Δ
LINEAR=(1-u)*p0+u*p1
SLERP=[sin((1-u)θ)*q0+sin(uθ)*q1]/sinθ
h00=2u³-3u²+1, h10=u³-2u²+u, h01=-2u³+3u², h11=u³-u²
CUBICSPLINE=h00*p0+h10*Δ*m0+h01*p1+h11*Δ*m1
J_j=joint_global_j*inverse_bind_j
p_skinned=Σj weight_j*(J_j*p_bind)
```

통계의 N은 표본 수이고 Amdahl의 N은 worker 수다. 서로 다른 식의 기호를 같은 변수로 혼동하지 않는다. [24장](24-디버그-뷰-테스트-프로파일링.md), [25장](25-타일링-worker-simd와-최종-capstone.md), [26장](26-glb-장면-skinning-animation.md)에 각각의 전제와 숫자 예제가 있다. SLERP의 부호 선택·작은 각도 처리와 CUBICSPLINE tangent의 단위는 26장의 전체 설명을 함께 읽는다.
