import math

def _webMercatorToWgs84(mercatorPoint):
    x = mercatorPoint[0]
    y = mercatorPoint[1]
    num4 = x / 6378137.0 * 57.295779513082323
    num5 = math.floor((num4 + 180.0) / 360.0)
    num6 = num4 - (num5 * 360.0)
    num7 = 1.5707963267948966 - (2.0 * math.atan(math.exp((-1.0 * y) / 6378137.0)))
    return (num7 * 57.295779513082323, num6)

def _wgs84ToWebMercator(wgsPoint):
    if abs(wgsPoint[0]) >= 90.0:
      return (math.inf, math.inf)
    num = wgsPoint[1] * 0.017453292519943295
    x = 6378137.0 * num
    a = wgsPoint[0] * 0.017453292519943295
    y = 3189068.5 * math.log((1.0 + math.sin(a)) / (1.0 - math.sin(a)))
    return (x, y)

class Transformer(object):
  def __init__(self, inverse):
    self._inverse = inverse

  def transform(self, x, y):
    if self._inverse:
      return _wgs84ToWebMercator((x, y))
    else:
      return _webMercatorToWgs84((x, y))

  @staticmethod
  def from_crs(proj_from, proj_to):
    if proj_from.lower() == 'epsg:3857' and proj_to.lower() == 'epsg:4326':
      return Transformer(False)
    if proj_from.lower() == 'epsg:4326' and proj_to.lower() == 'epsg:3857':
      return Transformer(True)
    raise ValueError('Unsupported projections')
