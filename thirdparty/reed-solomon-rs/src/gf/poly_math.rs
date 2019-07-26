use core::cmp::max;
use core::ops::DerefMut;
use ::gf::poly::Polynom;
use ::gf;

pub trait Scale<T: gf::GF2_X> {
    fn scale(&self, x: u8) -> Polynom;
    fn scale_assign(&mut self, x: u8) -> &mut Self;
}

pub trait Add<T: gf::GF2_X>  {
    fn add(&self, rhs: &Self) -> Polynom;
    fn add_assign(&mut self, rhs: &Self) -> &mut Self;
}

pub trait Mul<T: gf::GF2_X>  {
    fn mul(&self, rhs: &Self) -> Polynom;
}

pub trait Div<T: gf::GF2_X>  {
    fn div(&self, rhs: &Self) -> (Polynom, Polynom);
}

pub trait Eval<T: gf::GF2_X>  {
    fn eval(&self, x: u8) -> u8;
}

impl<T: gf::GF2_X> Scale<T> for [u8] {
    #[inline]
    fn scale(&self, x: u8) -> Polynom {
        let mut poly = Polynom::from(self);
        Scale::<T>::scale_assign(poly.deref_mut(), x);
        poly
    }

    #[inline]
    fn scale_assign(&mut self, x: u8) -> &mut Self {
        for px in self.iter_mut() {
            *px = T::mul(*px, x);
        }
        self
    }
}

impl<T: gf::GF2_X> Add<T> for [u8] {
    fn add(&self, rhs: &Self) -> Polynom {
        let mut poly = Polynom::with_length(max(self.len(), rhs.len()));

        for (i, x) in self.iter().enumerate() {
            let index = i + poly.len() - self.len();
            uncheck_mut!(poly[index]) = *x;
        }

        for (i, x) in rhs.iter().enumerate() {
            let index = i + poly.len() - rhs.len();
            uncheck_mut!(poly[index]) ^= *x;
        }

        poly
    }

    fn add_assign(&mut self, rhs: &Self) -> &mut Self {
        let poly = Add::<T>::add(self, rhs);
        self.copy_from_slice(&poly);
        self
    }
}

impl<T: gf::GF2_X> Mul<T> for [u8] {
    #[inline]
    fn mul(&self, rhs: &Self) -> Polynom {
        let mut poly = Polynom::with_length(self.len() + rhs.len() - 1);

        for (j, rhs_x) in rhs.iter().enumerate() {
            for (i, self_x) in self.iter().enumerate() {
                uncheck_mut!(poly[i + j]) ^= T::mul(*self_x, *rhs_x);
            }
        }

        poly
    }
}

impl<T: gf::GF2_X> Div<T> for [u8] {
    fn div(&self, rhs: &Self) -> (Polynom, Polynom) {
        let mut poly = Polynom::from(self);

        // If divisor's degree (len-1) is bigger, all dividend is a remainder
        let divisor_degree = rhs.len() - 1;
        if self.len() < divisor_degree {
            return (Polynom::new(), poly);
        }

        for i in 0..(self.len() - divisor_degree) {
            let coef = uncheck!(poly[i]);
            if coef != 0 {
                for j in 1..rhs.len() {
                    if rhs[j] != 0 {
                        uncheck_mut!(poly[i + j]) ^= T::mul(rhs[j], coef);
                    }
                }
            }
        }

        let separator = self.len() - (rhs.len() - 1);

        // Quotient is after separator
        let remainder = Polynom::from(&poly[separator..]);

        // And reminder is before separator, so just shrink to it
        poly.set_length(separator);

        (poly, remainder)
    }
}

impl<T: gf::GF2_X> Eval<T> for [u8] {
    #[inline]
    fn eval(&self, x: u8) -> u8 {
        let mut y = self[0];
        for px in self.iter().skip(1) {
            y = T::mul(y, x) ^ px;
        }
        y
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale() {
        let poly = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let answer = [0, 3, 6, 5, 12, 15, 10, 9, 24, 27];
        assert_eq!(answer, *(Scale::<gf::GF2_8>::scale(&poly as &[_], 3)));
    }

    #[test]
    fn scale_assign() {
        let mut poly = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let answer = [0, 3, 6, 5, 12, 15, 10, 9, 24, 27];
        assert_eq!(answer,
                   *({
                       Scale::<gf::GF2_8>::scale_assign(&mut poly as &mut [_], 3);
                       &poly
                   }));
    }

    #[test]
    fn add() {
        let px = [0u8, 5, 10, 15, 20];
        let py = [3, 9, 17, 24, 75];
        assert_eq!([3, 12, 27, 23, 95], *(Add::<gf::GF2_8>::add(&px as &[_], &py)));

        let px = [0u8, 5, 10];
        let py = [3u8, 9, 17, 24, 75];

        assert_eq!([3, 9, 17, 29, 65], *(Add::<gf::GF2_8>::add(&px as &[_], &py)));
        assert_eq!([3, 9, 17, 29, 65], *(Add::<gf::GF2_8>::add(&py as &[_], &px)))
    }

    #[test]
    fn mul() {
        let px = [0u8, 5, 10, 15, 20];
        let py = [3u8, 9, 17, 24, 75];
        assert_eq!([0, 15, 51, 30, 153, 193, 53, 115, 245], *(Mul::<gf::GF2_8>::mul(&px as &[_], &py)));

        let px = [0u8, 5, 10];
        let py = [3u8, 9, 17, 24, 75];

        assert_eq!([0, 15, 51, 15, 210, 138, 244], *(Mul::<gf::GF2_8>::mul(&px as &[_], &py)));
        assert_eq!([0, 15, 51, 15, 210, 138, 244], *(Mul::<gf::GF2_8>::mul(&py as &[_], &px)));
    }

    #[test]
    fn div() {
        let px = [0u8, 5, 10, 15, 20];
        let py = [3, 9, 17, 24, 75];

        let (q, r) = Div::<gf::GF2_8>::div(&px as &[_], &py);
        assert_eq!([0], *q);
        assert_eq!([5, 10, 15, 20], *r);

        let (q, r) = Div::<gf::GF2_8>::div(&py as &[_], &px);
        assert_eq!([3], *q);
        assert_eq!([6, 15, 9, 119], *r);

        let px = [0u8, 5, 10];
        let py = [3, 9, 17, 24, 75];

        let empty: [u8; 0] = [];
        let (q, r) = Div::<gf::GF2_8>::div(&px as &[_], &py);

        assert_eq!(empty, *q);
        assert_eq!([0, 5, 10], *r);

        let (q, r) = Div::<gf::GF2_8>::div(&py as &[_], &px);
        assert_eq!([3, 6, 17], *q);
        assert_eq!([113, 225], *r);
    }

    #[test]
    fn eval() {
        let p = [0u8, 5, 10, 15, 20];
        let tests = [4, 7, 21, 87, 35, 255];
        let answers = [213, 97, 132, 183, 244, 92];

        for i in 0..tests.len() {
            assert_eq!(answers[i], Eval::<gf::GF2_8>::eval(&p as &[_], tests[i]));
        }
    }
}
